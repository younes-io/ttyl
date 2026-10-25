mod common;

use base64ct::{Base64, Encoding};
use ttyl::desktop::osc52;

const URL: &str = "https://dev-null.sh/@grace/commits/2c3d4e5f-0000-4000-8000-000000000201";

#[test]
fn the_clipboard_sequence_carries_the_link_and_survives_tmux() {
    let encoded = Base64::encode_string(URL.as_bytes());
    assert_eq!(osc52(URL, false), format!("\x1b]52;c;{encoded}\x07"));
    assert_eq!(
        osc52(URL, true),
        format!("\x1b]52;c;{encoded}\x07\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\")
    );
}

#[cfg(target_os = "macos")]
mod pasteboard {
    use std::path::PathBuf;
    use std::process::Command;

    use ttyl::desktop::{Clipped, pasteboard};

    use crate::common::scratch;

    const BOARD: &str = "ttyl-test";

    /// Puts `kind` on the test pasteboard, so the real clipboard is left alone.
    fn put(kind: &str, value: &str) {
        let script = format!(
            "ObjC.import('AppKit');
            function run(argv) {{
              const board = $.NSPasteboard.pasteboardWithName('{BOARD}');
              board.clearContents;
              const [kind, value] = argv;
              if (kind === 'text') board.setStringForType($(value), $.NSPasteboardTypeString);
              if (kind === 'files') board.writeObjects($(value.split('\\n').map(path => $.NSURL.fileURLWithPath(path))));
              if (kind === 'tiff') board.setDataForType($.NSImage.alloc.initWithContentsOfFile(value).TIFFRepresentation, 'public.tiff');
              return '';
            }}"
        );
        let status = Command::new("osascript")
            .args(["-l", "JavaScript", "-e", &script, kind, value])
            .output()
            .expect("osascript runs")
            .status;
        assert!(status.success(), "put {kind} on the test pasteboard");
    }

    #[test]
    fn text_files_and_images_come_back_from_the_pasteboard() {
        put("text", "héllo\nwörld");
        assert_eq!(
            pasteboard(Some(BOARD)),
            Clipped::Text("héllo\nwörld".to_owned())
        );

        let dir = scratch("pasteboard");
        let spaced = dir.join("my pic.png");
        let plain = dir.join("b.png");
        let paths = [spaced.clone(), plain.clone()];
        for path in &paths {
            std::fs::write(path, b"").unwrap();
        }
        put(
            "files",
            &format!("{}\n{}", spaced.display(), plain.display()),
        );
        let Clipped::Files(got) = pasteboard(Some(BOARD)) else {
            panic!("a Finder copy reads as files");
        };
        let canonical = |path: &PathBuf| path.canonicalize().unwrap();
        assert_eq!(
            got.iter().map(canonical).collect::<Vec<_>>(),
            paths.iter().map(canonical).collect::<Vec<_>>()
        );

        let picture = dir.join("picture.png");
        image::RgbImage::new(4, 4).save(&picture).unwrap();
        put("tiff", picture.to_str().unwrap());
        let Clipped::Image(bytes) = pasteboard(Some(BOARD)) else {
            panic!("a copied picture reads as an image");
        };
        assert!(
            bytes.starts_with(b"\x89PNG"),
            "the picture arrives as a PNG"
        );

        put("text", "");
        assert_eq!(pasteboard(Some(BOARD)), Clipped::Empty);
    }
}
