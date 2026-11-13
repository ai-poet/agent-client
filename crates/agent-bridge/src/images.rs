//! The pictures a prompt names, sent beside its text.
//!
//! The desktop gives every agent the same prompt: the text, then one `@path`
//! per attachment (`merged_submission` in its composer). Claude Code and
//! Codex open those paths themselves; the engine does not — its file reader
//! answers an image with a line of text — so a pasted screenshot reached the
//! model as a file name. The bridge reads the pictures a prompt mentions and
//! sends them as image blocks, whatever the model: one that cannot take
//! images says so in the API's error, which is the gateway's to decide.

use std::path::{Path, PathBuf};

use agent_core::types::{ContentBlock, ImageSource, Message};
use base64::Engine as _;

/// Past what any image API accepts. A bigger file stays a path rather than
/// a payload every later request of the session would carry.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Shorter readings of one mention tried at most: a mention runs to the next
/// one, and prose may follow the last.
const MAX_CUTS: usize = 32;

/// The user message for `prompt`: its text alone, or — when it names
/// pictures — those pictures, then the text.
pub(crate) fn user_message(prompt: String, cwd: &Path) -> Message {
    let mut blocks = prompt_images(&prompt, cwd);
    if blocks.is_empty() {
        return Message::user(prompt);
    }
    blocks.push(ContentBlock::Text { text: prompt });
    Message::user_blocks(blocks)
}

/// An image block for each picture `prompt` mentions, in order, once each.
fn prompt_images(prompt: &str, cwd: &Path) -> Vec<ContentBlock> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut blocks = Vec::new();
    for mention in mentions(prompt) {
        let Some(path) = picture_path(mention, cwd) else {
            continue;
        };
        if seen.contains(&path) {
            continue;
        }
        if let Some(block) = image_block(&path) {
            seen.push(path);
            blocks.push(block);
        }
    }
    blocks
}

/// What follows each `@` that starts a word, up to the next such `@`. An
/// address like `a@b.com` is no mention.
fn mentions(prompt: &str) -> Vec<&str> {
    let starts: Vec<usize> = prompt
        .char_indices()
        .filter(|&(index, ch)| {
            ch == '@'
                && prompt[..index]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace)
        })
        .map(|(index, _)| index)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let end = starts.get(n + 1).copied().unwrap_or(prompt.len());
            prompt[start + 1..end].trim_end()
        })
        .filter(|mention| !mention.is_empty())
        .collect()
}

/// The picture a mention names: the whole of it when that is one, else the
/// longest reading cut at a space — paths are not quoted, so one with a
/// space in it and one followed by more words look alike until the disk
/// says which exists. Relative paths are the session's working directory's.
fn picture_path(mention: &str, cwd: &Path) -> Option<PathBuf> {
    let mut candidate = mention;
    for _ in 0..MAX_CUTS {
        let name = candidate.trim_matches('"').trim_end_matches([
            ',', '.', ';', ':', ')', '!', '?', '，', '。', '；', '：', '）', '！', '？',
        ]);
        if has_picture_extension(name) {
            let path = Path::new(name);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            if path.is_file() {
                return Some(path);
            }
        }
        let (shorter, _) = candidate.rsplit_once(char::is_whitespace)?;
        candidate = shorter.trim_end();
        if candidate.is_empty() {
            return None;
        }
    }
    None
}

/// The raster formats image APIs read directly, plus local formats we can
/// losslessly transcode to PNG before constructing the content block.
fn has_picture_extension(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "bmp"
                    | "ico"
                    | "tif"
                    | "tiff"
                    | "pnm"
                    | "pbm"
                    | "pgm"
                    | "ppm"
            )
        })
}

fn image_block(path: &Path) -> Option<ContentBlock> {
    let size = std::fs::metadata(path).ok()?.len();
    if size == 0 || size > MAX_IMAGE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let (bytes, media_type) = match media_type(&bytes) {
        Some(media_type) => (bytes, media_type),
        None => {
            let decoded = image::load_from_memory(&bytes).ok()?;
            let mut encoded = std::io::Cursor::new(Vec::new());
            decoded
                .write_to(&mut encoded, image::ImageFormat::Png)
                .ok()?;
            let bytes = encoded.into_inner();
            if bytes.len() as u64 > MAX_IMAGE_BYTES {
                return None;
            }
            (bytes, "image/png")
        }
    };
    Some(ContentBlock::Image {
        source: ImageSource {
            source_type: "base64".to_owned(),
            media_type: Some(media_type.to_owned()),
            data: Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
            url: None,
        },
    })
}

/// The format the bytes are in, whatever the name says: Anthropic refuses an
/// image whose declared type is not its real one, and a `.png` saved from a
/// browser is often a JPEG.
fn media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::types::MessageContent;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];

    fn media_types(message: &Message) -> Vec<String> {
        let MessageContent::Blocks(blocks) = &message.content else {
            return Vec::new();
        };
        blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Image { source } => source.media_type.clone(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn mentions_run_to_the_next_one_and_skip_addresses() {
        assert_eq!(
            mentions("compare mail@example.com @a b.png @c.jpg"),
            ["a b.png", "c.jpg"]
        );
        assert!(mentions("no pictures here").is_empty());
        assert_eq!(mentions("@shot.png"), ["shot.png"]);
    }

    /// The composer's shape — text, then one mention per attachment — with
    /// a pasted image outside the project (absolute) and one inside it
    /// (relative, with a space in its name).
    #[test]
    fn attached_pictures_go_before_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let pasted = dir.path().join("pasted.png");
        std::fs::write(&pasted, PNG).unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs").join("my shot.jpg"), JPEG).unwrap();

        let prompt = format!("what is wrong here @{} @docs/my shot.jpg", pasted.display());
        let message = user_message(prompt.clone(), dir.path());
        assert_eq!(media_types(&message), ["image/png", "image/jpeg"]);
        let MessageContent::Blocks(blocks) = &message.content else {
            panic!("expected blocks");
        };
        assert!(matches!(blocks.last(), Some(ContentBlock::Text { text }) if *text == prompt));
    }

    /// Prose after a mention the user typed, and a name whose extension
    /// lies about the format.
    #[test]
    fn a_typed_mention_is_found_and_its_real_format_sent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("photo.png"), JPEG).unwrap();
        let message = user_message("see @photo.png, then fix the layout".into(), dir.path());
        assert_eq!(media_types(&message), ["image/jpeg"]);
    }

    #[test]
    fn bmp_attachments_are_sent_as_real_png_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.bmp");
        let mut bmp = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 1)
            .write_to(&mut bmp, image::ImageFormat::Bmp)
            .unwrap();
        std::fs::write(&path, bmp.into_inner()).unwrap();

        let message = user_message("what does @picture.bmp say?".into(), dir.path());
        let MessageContent::Blocks(blocks) = &message.content else {
            panic!("expected image blocks");
        };
        let ContentBlock::Image { source } = &blocks[0] else {
            panic!("expected an image first");
        };
        assert_eq!(source.media_type.as_deref(), Some("image/png"));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(source.data.as_deref().unwrap())
            .unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::Png
        );
    }

    #[test]
    fn a_prompt_without_pictures_stays_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hi").unwrap();
        std::fs::write(dir.path().join("fake.png"), "not a picture").unwrap();
        for prompt in [
            "look at @notes.txt",
            "look at @missing.png",
            "look at @fake.png",
        ] {
            let message = user_message(prompt.into(), dir.path());
            assert!(
                matches!(message.content, MessageContent::Text(_)),
                "{prompt}"
            );
        }
    }

    #[test]
    fn one_picture_mentioned_twice_is_sent_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), PNG).unwrap();
        let message = user_message("@a.png and again @a.png".into(), dir.path());
        assert_eq!(media_types(&message), ["image/png"]);
    }
}
