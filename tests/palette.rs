use std::fs;
use std::path::Path;

/// Every color on screen is a theme token, so a new one is a choice made in `src/theme.rs` and
/// every screen keeps one palette.
#[test]
fn colors_come_from_the_theme() {
    let mut strays = Vec::new();
    let mut dirs = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.ends_with("theme.rs") {
                continue;
            } else if let Ok(source) = fs::read_to_string(&path) {
                for (number, line) in source.lines().enumerate() {
                    if line.contains("Color::") {
                        strays.push(format!(
                            "{}:{}: {}",
                            path.display(),
                            number + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        strays.is_empty(),
        "name these colors in src/theme.rs:\n{}",
        strays.join("\n")
    );
}
