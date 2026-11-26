//! Best-effort, UI-free media metadata. Optional tools are never required.
use image::ImageDecoder;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};
use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub type Fields = BTreeMap<String, String>;

#[derive(Clone, Debug, Default)]
pub struct Tools {
    pub exiftool: Option<PathBuf>,
    pub video_probe: Option<PathBuf>,
    pub office_text: Option<PathBuf>,
}

fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}

pub fn is_audio(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "mp3" | "flac" | "ogg" | "oga" | "m4a" | "opus" | "wav" | "aiff" | "ape"
    )
}

pub fn is_image(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff"
    )
}

pub fn is_video(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v" | "flv" | "wmv"
    )
}

pub fn is_office(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "docx" | "xlsx" | "pptx" | "ods" | "odt" | "odp"
    )
}

pub fn is_kdbx(path: &Path) -> bool {
    extension(path) == "kdbx"
}

pub fn file_kind(path: &Path, is_text: Option<bool>) -> &'static str {
    if is_kdbx(path) {
        "kdbx"
    } else if is_audio(path) {
        "audio"
    } else if is_image(path) {
        "image"
    } else if is_video(path) {
        "video"
    } else if is_office(path) {
        "office"
    } else if is_text == Some(false) {
        "other"
    } else {
        "text"
    }
}

pub fn media_supported(path: &Path) -> bool {
    is_audio(path) || is_image(path) || is_video(path) || is_office(path)
}

// Drain the pipe concurrently, but retain only bounded output. Kill stalled tools.
fn run_tool(tool: &Path, args: &[&str], path: &Path) -> Option<String> {
    let mut child = Command::new(tool)
        .args(args)
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0; 8192];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let take = n.min(1024 * 1024usize - kept.len());
                    kept.extend_from_slice(&buf[..take]);
                }
            }
        }
        kept
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() < Duration::from_secs(15) => {
                thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // A descendant may inherit stdout, so do not wait indefinitely for the reader.
    let deadline = Instant::now() + Duration::from_millis(250);
    while !reader.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    if !reader.is_finished() {
        return None;
    }
    let bytes = reader.join().ok()?;
    if !status?.success() {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn exif_key(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let count = chars.iter().take_while(|c| c.is_ascii_uppercase()).count();
    let lower = if count > 1 { count - 1 } else { count };
    chars[..lower]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase()
        + &chars[lower..].iter().collect::<String>()
}

fn exif_fields(path: &Path, tool: Option<&Path>, image: bool) -> Option<Fields> {
    let output = run_tool(tool?, &["-s", "-j"], path)?;
    let parsed: serde_json::Value = serde_json::from_str(&output).ok()?;
    let tags = parsed.as_array()?.first()?.as_object()?;
    let skip = [
        "SourceFile",
        "ExifToolVersion",
        "FileName",
        "Directory",
        "FileSize",
        "FileModifyDate",
        "FileAccessDate",
        "FileInodeChangeDate",
        "FilePermissions",
        "FileTypeExtension",
        "ImageSize",
        "Megapixels",
        "FileType",
        "ImageWidth",
        "ImageHeight",
        "ThumbnailImage",
        "ThumbnailLength",
        "ThumbnailOffset",
        "PreviewImage",
        "PreviewImageLength",
        "PreviewImageStart",
        "MakerNoteApple",
        "MakerNoteCanon",
        "MakerNoteNikon",
        "MakerNoteSony",
        "MakerNotes",
    ];
    let mut fields = Fields::new();
    let (mut width, mut height) = (None, None);
    for (key, raw_value) in tags {
        let Some(value) = exif_value(raw_value) else {
            continue;
        };
        if key == "ImageWidth" {
            width = Some(value.clone());
        }
        if key == "ImageHeight" {
            height = Some(value.clone());
        }
        if skip.contains(&key.as_str())
            || value.is_empty()
            || value.chars().count() > 200
            || value.starts_with("(Binary data")
        {
            continue;
        }
        fields.insert(exif_key(key), value);
    }
    if image {
        if let (Some(w), Some(h)) = (width.as_deref(), height.as_deref()) {
            fields.insert("dimensions".into(), format!("{w}x{h}"));
        }
    }
    (!fields.is_empty()).then_some(fields)
}

fn exif_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        }
        serde_json::Value::Array(values) => {
            let values: Vec<_> = values.iter().filter_map(exif_value).collect();
            (!values.is_empty()).then(|| values.join(", "))
        }
        serde_json::Value::Null | serde_json::Value::Object(_) => None,
    }
}

fn image_fields(path: &Path) -> io::Result<Fields> {
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let format = reader
        .format()
        .ok_or_else(|| io::Error::other("unknown image format"))?;
    let mut decoder = reader.into_decoder().map_err(io::Error::other)?;
    let (w, h) = decoder.dimensions();
    let mut fields = Fields::from([
        ("dimensions".into(), format!("{w}x{h}")),
        ("format".into(), format!("{format:?}").to_uppercase()),
    ]);
    if let Some(exif) = decoder.exif_metadata().map_err(io::Error::other)? {
        fields.insert("exif_sha256".into(), format!("{:x}", Sha256::digest(exif)));
    }
    Ok(fields)
}

fn audio_fields(path: &Path) -> Option<Fields> {
    let audio = Probe::open(path).ok()?.read().ok()?;
    let mut fields = Fields::new();
    if let Some(tag) = audio.primary_tag().or_else(|| audio.first_tag()) {
        for (key, value) in [
            ("title", tag.title()),
            ("artist", tag.artist()),
            ("album", tag.album()),
            (
                "date",
                tag.get_string(ItemKey::RecordingDate).map(Into::into),
            ),
            ("genre", tag.genre()),
            ("tracknumber", tag.track().map(|n| n.to_string().into())),
        ] {
            if let Some(value) = value {
                fields.insert(key.into(), value.into_owned());
            }
        }
    }
    if fields.is_empty() {
        fields.insert(
            "length".into(),
            audio.properties().duration().as_secs_f64().to_string(),
        );
    }
    Some(fields)
}

// Bound decompressed XML to avoid excessive allocation on untrusted archives.
fn office_xml(zip: &mut zip::ZipArchive<File>, name: &str) -> Option<String> {
    let entry = zip.by_name(name).ok()?;
    if entry.size() > 8 * 1024 * 1024 {
        return None;
    }
    let mut bytes = Vec::new();
    entry
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    String::from_utf8(bytes).ok()
}

// Return completed elements with their descendant text and the spreadsheet cell type.
fn xml_elements(xml: &str) -> Option<Vec<(String, String, String)>> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<(String, String, String)> = Vec::new();
    let mut result = Vec::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => {
                if stack.len() >= 32 {
                    return None;
                }
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let mut typ = String::new();
                for attr in e.attributes() {
                    let attr = attr.ok()?;
                    if attr.key.as_ref() == b"t" {
                        typ = attr
                            .decode_and_unescape_value(reader.decoder())
                            .ok()?
                            .into_owned();
                    }
                }
                stack.push((name, String::new(), typ));
            }
            Event::Text(e) => {
                let decoded = e.xml_content().ok()?;
                let text = quick_xml::escape::unescape(&decoded).ok()?;
                if let Some(item) = stack.last_mut() {
                    item.1.push_str(&text);
                }
            }
            Event::CData(e) => {
                if let Some(item) = stack.last_mut() {
                    item.1.push_str(&e.xml_content().ok()?);
                }
            }
            Event::GeneralRef(e) => {
                let reference = format!("&{};", e.decode().ok()?);
                let text = quick_xml::escape::unescape(&reference).ok()?;
                if let Some(item) = stack.last_mut() {
                    item.1.push_str(&text);
                }
            }
            Event::End(_) => {
                let item = stack.pop()?;
                if let Some(parent) = stack.last_mut() {
                    parent.1.push_str(&item.1);
                }
                result.push(item);
            }
            Event::Eof => return stack.is_empty().then_some(result),
            _ => {}
        }
    }
}

fn office_fields(path: &Path, tools: &Tools) -> Option<Fields> {
    let mut zip = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
    let mut fields = Fields::new();
    if let Some(xml) = office_xml(&mut zip, "docProps/core.xml") {
        if let Some(elements) = xml_elements(&xml) {
            for (name, value, _) in elements {
                let key = match name.as_str() {
                    "title" | "subject" | "creator" | "description" | "created" | "modified" => {
                        name.as_str()
                    }
                    "lastModifiedBy" => "last_modified_by",
                    _ => continue,
                };
                if !value.trim().is_empty() {
                    fields.entry(key.into()).or_insert(value.trim().into());
                }
            }
        }
    }
    let external = tools.office_text.as_deref().and_then(|tool| {
        let pandoc = tool
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .starts_with("pandoc");
        run_tool(tool, if pandoc { &["-t", "plain"] } else { &[] }, path)
            .filter(|s| !s.trim().is_empty())
    });
    let content = if let Some(text) = external {
        text.trim().to_owned()
    } else {
        let mut names: Vec<String> = zip
            .file_names()
            .map(|n| n.map(|s| s.into_owned()))
            .collect::<Result<_, _>>()
            .ok()?;
        names.sort();
        let mut parts = Vec::new();
        let shared = if names.iter().any(|name| name == "xl/sharedStrings.xml") {
            let xml = office_xml(&mut zip, "xl/sharedStrings.xml")?;
            xml_elements(&xml)?
                .into_iter()
                .filter(|e| e.0 == "si")
                .map(|e| e.1)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for name in names {
            let sheet = name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml");
            if name != "content.xml"
                && name != "word/document.xml"
                && !sheet
                && !(name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
            {
                continue;
            }
            let xml = office_xml(&mut zip, &name)?;
            let elements = xml_elements(&xml)?;
            if sheet {
                let vals: Vec<String> = elements
                    .into_iter()
                    .filter(|e| e.0 == "c")
                    .map(|(_, v, t)| {
                        if t == "s" {
                            v.trim()
                                .parse::<usize>()
                                .ok()
                                .and_then(|i| shared.get(i))
                                .cloned()
                        } else {
                            Some(v)
                        }
                    })
                    .collect::<Option<Vec<_>>>()?
                    .into_iter()
                    .filter(|v| !v.trim().is_empty())
                    .collect();
                if !vals.is_empty() {
                    parts.push(vals.join(", "));
                }
            } else {
                parts.extend(
                    elements
                        .into_iter()
                        .filter(|e| e.0 == "p" && !e.1.trim().is_empty())
                        .map(|e| e.1),
                );
            }
        }
        if parts.is_empty() {
            "<no text>".into()
        } else {
            parts.join("\n")
        }
    };
    fields.insert("content".into(), content.chars().take(4000).collect());
    Some(fields)
}

pub fn video_summary(path: &Path, ffprobe: Option<&Path>) -> String {
    let Some(tool) = ffprobe else {
        return "<unknown video>".into();
    };
    let Some(output) = run_tool(
        tool,
        &[
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,width,height",
            "-of",
            "default",
        ],
        path,
    ) else {
        return "<ffprobe failed>".into();
    };
    let mut parts = Vec::new();
    let mut stream = Fields::new();
    for line in output.lines() {
        if line == "[STREAM]" {
            stream.clear();
        } else if line == "[/STREAM]" {
            let mut item = format!(
                "{}:{}",
                stream.get("codec_type").map_or("None", String::as_str),
                stream.get("codec_name").map_or("None", String::as_str)
            );
            if let Some(w) = stream.get("width").filter(|w| w.as_str() != "0") {
                item.push_str(&format!(
                    " {w}x{}",
                    stream.get("height").map_or("None", String::as_str)
                ));
            }
            parts.push(item);
        } else if let Some((k, v)) = line.split_once('=') {
            stream.insert(k.into(), v.into());
        }
    }
    if parts.is_empty() {
        "<no streams>".into()
    } else {
        parts.join(", ")
    }
}

/// Returns None when extraction is unavailable or failed, rather than asserting equality.
pub fn content_fields(path: &Path, kind: &str, tools: &Tools) -> Option<Fields> {
    match kind {
        "audio" => {
            exif_fields(path, tools.exiftool.as_deref(), false).or_else(|| audio_fields(path))
        }
        "image" => {
            exif_fields(path, tools.exiftool.as_deref(), true).or_else(|| image_fields(path).ok())
        }
        "video" => {
            let value = video_summary(path, tools.video_probe.as_deref());
            if value.starts_with('<') && value != "<no streams>" {
                None
            } else {
                Some(Fields::from([("streams".into(), value)]))
            }
        }
        "office" => office_fields(path, tools),
        _ => None,
    }
}

/// Compares extracted metadata only, not image pixels or file contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataComparison {
    Unavailable,
    Equal,
    Different(String),
}

pub fn metadata_comparison(a: &Path, b: &Path, kind: &str, tools: &Tools) -> MetadataComparison {
    let (Some(a), Some(b)) = (
        content_fields(a, kind, tools),
        content_fields(b, kind, tools),
    ) else {
        return MetadataComparison::Unavailable;
    };
    metadata_comparison_fields(&a, &b, kind)
}

pub(crate) fn metadata_comparison_fields(a: &Fields, b: &Fields, kind: &str) -> MetadataComparison {
    let keys: BTreeSet<_> = a
        .keys()
        .chain(b.keys())
        .filter(|k| a.get(*k) != b.get(*k))
        .collect();
    if keys.is_empty() {
        return MetadataComparison::Equal;
    }
    if kind == "audio" {
        MetadataComparison::Different(format!(
            "tags differ ({})",
            keys.iter()
                .map(|k| format!(
                    "{k}: {} → {}",
                    a.get(*k).map_or("∅", String::as_str),
                    b.get(*k).map_or("∅", String::as_str)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    } else {
        MetadataComparison::Different(format!(
            "metadata differs ({})",
            keys.into_iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// Returns `None` when metadata extraction is unavailable.
pub fn metadata_equal(a: &Path, b: &Path, kind: &str, tools: &Tools) -> Option<bool> {
    match metadata_comparison(a, b, kind, tools) {
        MetadataComparison::Unavailable => None,
        MetadataComparison::Equal => Some(true),
        MetadataComparison::Different(_) => Some(false),
    }
}

/// Legacy convenience API: `None` means equal or unavailable. Use `metadata_comparison` to distinguish them.
pub fn metadata_diff(a: &Path, b: &Path, kind: &str, tools: &Tools) -> Option<String> {
    match metadata_comparison(a, b, kind, tools) {
        MetadataComparison::Different(diff) => Some(diff),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_matches_python() {
        assert_eq!(file_kind(Path::new("x.MP3"), Some(true)), "audio");
        assert_eq!(file_kind(Path::new("x.kdbx"), None), "kdbx");
        assert_eq!(file_kind(Path::new("x.bin"), Some(false)), "other");
        assert_eq!(file_kind(Path::new("x"), None), "text");
        assert!(media_supported(Path::new("x.ODT")));
    }

    #[test]
    fn image_headers_and_unknown_are_conservative() {
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("deconflict-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.png");
        let b = dir.join("b.png");
        image::RgbImage::new(2, 3).save(&a).unwrap();
        image::RgbImage::new(2, 4).save(&b).unwrap();
        let tools = Tools::default();
        assert_eq!(
            content_fields(&a, "image", &tools).unwrap()["dimensions"],
            "2x3"
        );
        assert_eq!(metadata_equal(&a, &b, "image", &tools), Some(false));
        assert!(metadata_diff(&a, &b, "image", &tools)
            .unwrap()
            .contains("dimensions"));
        image::RgbImage::from_pixel(2, 3, image::Rgb([255, 0, 0]))
            .save(&b)
            .unwrap();
        assert_eq!(metadata_equal(&a, &b, "image", &tools), Some(true));
        assert_ne!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
        assert_eq!(metadata_equal(&a, &b, "audio", &tools), None);
        std::fs::write(&a, b"broken").unwrap();
        assert_eq!(metadata_equal(&a, &a, "image", &tools), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_tools_and_invalid_kinds_are_safe() {
        let path = Path::new("missing.mp4");
        let tools = Tools {
            video_probe: Some("/nonexistent/ffprobe".into()),
            ..Tools::default()
        };
        assert_eq!(video_summary(path, None), "<unknown video>");
        assert_eq!(
            video_summary(path, tools.video_probe.as_deref()),
            "<ffprobe failed>"
        );
        assert_eq!(metadata_equal(path, path, "video", &tools), None);
        assert_eq!(metadata_equal(path, path, "invalid", &tools), None);
        assert_eq!(metadata_equal(path, path, "text", &tools), None);
        assert_eq!(
            metadata_comparison(path, path, "video", &tools),
            MetadataComparison::Unavailable
        );
        assert_eq!(exif_key("MIMEType"), "mimeType");
        assert_eq!(exif_key("ID3Size"), "iD3Size");
    }

    #[test]
    fn comparison_distinguishes_equal_different_and_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");
        image::RgbImage::new(2, 3).save(&a).unwrap();
        image::RgbImage::new(2, 3).save(&b).unwrap();
        let tools = Tools::default();
        assert_eq!(
            metadata_comparison(&a, &b, "image", &tools),
            MetadataComparison::Equal
        );
        image::RgbImage::new(3, 3).save(&b).unwrap();
        assert!(matches!(
            metadata_comparison(&a, &b, "image", &tools),
            MetadataComparison::Different(_)
        ));
        std::fs::write(&b, b"broken").unwrap();
        assert_eq!(
            metadata_comparison(&a, &b, "image", &tools),
            MetadataComparison::Unavailable
        );
    }

    #[test]
    fn native_image_exif_changes_are_detected() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let b = dir.path().join("b.jpg");
        image::RgbImage::new(2, 2).save(&a).unwrap();
        let original = std::fs::read(&a).unwrap();
        let make_exif = |orientation: u8| {
            let mut jpeg = original[..2].to_vec();
            jpeg.extend_from_slice(&[
                0xff,
                0xe1,
                0,
                34,
                b'E',
                b'x',
                b'i',
                b'f',
                0,
                0,
                b'I',
                b'I',
                42,
                0,
                8,
                0,
                0,
                0,
                1,
                0,
                0x12,
                1,
                3,
                0,
                1,
                0,
                0,
                0,
                orientation,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]);
            jpeg.extend_from_slice(&original[2..]);
            jpeg
        };
        std::fs::write(&a, make_exif(1)).unwrap();
        std::fs::write(&b, make_exif(6)).unwrap();
        let tools = Tools::default();
        assert!(content_fields(&a, "image", &tools)
            .unwrap()
            .contains_key("exif_sha256"));
        assert!(matches!(
            metadata_comparison(&a, &b, "image", &tools),
            MetadataComparison::Different(_)
        ));
    }

    #[test]
    fn office_extracts_properties_text_and_spreadsheet_values() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.docx");
        let write_zip = |path: &Path, entries: &[(&str, &str)]| {
            let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
            for (name, data) in entries {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(data.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        };
        write_zip(
            &path,
            &[
                (
                    "docProps/core.xml",
                    "<core><title> Example &amp; More </title><creator>A</creator></core>",
                ),
                (
                    "word/document.xml",
                    "<doc><p><r>Hello &amp; <t>world</t></r></p><p>Second</p></doc>",
                ),
            ],
        );
        let fields = content_fields(&path, "office", &Tools::default()).unwrap();
        assert_eq!(fields["title"], "Example & More");
        assert_eq!(fields["creator"], "A");
        assert_eq!(fields["content"], "Hello & world\nSecond");
        let sheet = dir.path().join("sheet.xlsx");
        write_zip(&sheet, &[
            ("xl/sharedStrings.xml", "<sst><si><t>Apple</t></si></sst>"),
            ("xl/worksheets/sheet1.xml", "<sheet><c t=\"s\"><v>0</v></c><c><v>7</v></c><c t=\"inlineStr\"><is><t>hello</t></is></c></sheet>"),
        ]);
        assert_eq!(
            content_fields(&sheet, "office", &Tools::default()).unwrap()["content"],
            "Apple, 7, hello"
        );
        std::fs::write(&sheet, b"not a zip").unwrap();
        assert_eq!(content_fields(&sheet, "office", &Tools::default()), None);
    }

    #[test]
    fn native_media_reads_real_fixtures() {
        let root = std::env::current_dir()
            .unwrap()
            .ancestors()
            .find(|p| p.join("sample-files").is_dir())
            .unwrap()
            .to_path_buf();
        let samples = root.join("sample-files");
        let audio = samples.join("audio/small-glass-pane-shatter.mp3");
        let copy =
            samples.join("audio/small-glass-pane-shatter (conflicted copy 2026-09-19-105844).mp3");
        let tools = Tools::default();
        assert!(content_fields(&audio, "audio", &tools)
            .unwrap()
            .contains_key("title"));
        assert_eq!(metadata_equal(&audio, &copy, "audio", &tools), Some(false));
        let image = samples.join("img/IMG-picsum.photos-30-200x300.jpg");
        assert_eq!(
            content_fields(&image, "image", &tools).unwrap()["dimensions"],
            "200x300"
        );
        for name in [
            "sample.docx",
            "sample.odt",
            "cost table.xlsx",
            "cost table.ods",
        ] {
            let path = samples.join("office").join(name);
            let fields = content_fields(&path, "office", &tools).unwrap();
            assert_ne!(fields["content"], "<no text>", "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn optional_tools_extract_and_filter_metadata() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("deconflict-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("tool");
        std::fs::write(
            &tool,
            "#!/bin/sh\nprintf '[{\"FileName\":\"ignored\",\"ImageWidth\":42,\"ImageHeight\":21,\"MIMEType\":\"image/png\"}]\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tools = Tools {
            exiftool: Some(tool.clone()),
            ..Tools::default()
        };
        let fields = content_fields(Path::new("x.png"), "image", &tools).unwrap();
        assert_eq!(fields["dimensions"], "42x21");
        assert_eq!(fields["mimeType"], "image/png");
        assert!(!fields.contains_key("fileName"));
        std::fs::write(
            &tool,
            "#!/bin/sh\n[ \"$1\" = \"-s\" ] && [ \"$2\" = \"-j\" ] || exit 1\nprintf '[{\"MIMEType\":\"audio/mpeg\",\"MPEGAudioVersion\":1,\"ID3Size\":281,\"Comment\":\"I added a comment\",\"RecordingTime\":\"2026:09:19\"}]\\n'\n",
        )
        .unwrap();
        let fields = content_fields(Path::new("x.mp3"), "audio", &tools).unwrap();
        assert_eq!(fields["mimeType"], "audio/mpeg");
        assert_eq!(fields["mpegAudioVersion"], "1");
        assert_eq!(fields["iD3Size"], "281");
        assert_eq!(fields["comment"], "I added a comment");
        assert_eq!(fields["recordingTime"], "2026:09:19");
        std::fs::write(&tool, "#!/bin/sh\nprintf '[STREAM]\\ncodec_name=h264\\ncodec_type=video\\nwidth=42\\nheight=21\\n[/STREAM]\\n'\n").unwrap();
        assert_eq!(
            video_summary(Path::new("x.mp4"), Some(&tool)),
            "video:h264 42x21"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
