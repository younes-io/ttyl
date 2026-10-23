use ttyl::attach::{self, Kind, Probe};

fn sized(kind: Kind, width: u32, height: u32) -> Option<Probe> {
    Some(Probe {
        kind,
        width,
        height,
    })
}

#[test]
fn a_png_is_sized_from_its_header() {
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x07\x80\0\0\x04\x38\x08\x06";
    assert_eq!(attach::probe(png), sized(Kind::Png, 1920, 1080));
}

#[test]
fn a_jpeg_is_sized_past_its_metadata_segments() {
    let mut jpeg = vec![0xff, 0xd8];
    jpeg.extend([0xff, 0xe1, 0x00, 0x08, b'E', b'x', b'i', b'f', 0, 0]);
    jpeg.extend([0xff, 0xdb, 0x00, 0x03, 0x00]);
    // Fill bytes may pad the gap before a marker.
    jpeg.extend([
        0xff, 0xff, 0xc2, 0x00, 0x11, 0x08, 0x02, 0xd0, 0x05, 0x00, 0x03,
    ]);
    assert_eq!(attach::probe(&jpeg), sized(Kind::Jpeg, 1280, 720));
}

#[test]
fn a_jpeg_with_no_frame_before_its_scan_is_not_read() {
    let jpeg = [0xff, 0xd8, 0xff, 0xda, 0x00, 0x02, 0x00];
    assert_eq!(attach::probe(&jpeg), None);
}

#[test]
fn each_webp_flavour_is_sized() {
    let riff = |chunk: &[u8]| [b"RIFF\0\0\0\0WEBP".as_slice(), chunk].concat();
    let lossy = riff(b"VP8 \0\0\0\0\0\0\0\x9d\x01\x2a\x20\x03\x58\x02");
    assert_eq!(attach::probe(&lossy), sized(Kind::Webp, 800, 600));
    // 14 bits of width minus one, then 14 of height minus one: 640 by 480.
    let bits: u32 = 639 | (479 << 14);
    let lossless = riff(&[b"VP8L\0\0\0\0\x2f".as_slice(), &bits.to_le_bytes()].concat());
    assert_eq!(attach::probe(&lossless), sized(Kind::Webp, 640, 480));
    let extended = riff(b"VP8X\0\0\0\0\x10\0\0\0\x3f\x1f\0\x6f\x17\0");
    assert_eq!(attach::probe(&extended), sized(Kind::Webp, 8000, 6000));
}

#[test]
fn other_files_and_cut_headers_are_not_images() {
    assert_eq!(attach::probe(b"GIF89a\x01\0\x01\0"), None);
    assert_eq!(attach::probe(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0"), None);
    assert_eq!(attach::probe(b"RIFF\0\0\0\0WEBPVP8 "), None);
    assert_eq!(attach::probe(&[0xff, 0xd8, 0xff]), None);
    assert_eq!(attach::probe(b""), None);
}

#[test]
fn a_file_is_refused_for_what_the_server_would_refuse() {
    let shot = sized(Kind::Png, 6000, 3000);
    assert_eq!(attach::file_refusal(shot, attach::MAX_BYTES), None);
    assert_eq!(
        attach::file_refusal(shot, attach::MAX_BYTES + 1).as_deref(),
        Some("over 10 MB.")
    );
    assert_eq!(
        attach::file_refusal(sized(Kind::Webp, 8000, 6000), 1).as_deref(),
        Some("over 6000 px a side.")
    );
    assert_eq!(
        attach::file_refusal(None, 1).as_deref(),
        Some("not a jpeg, png or webp.")
    );
}
