use std::process::{Command, Output, Stdio};

fn ttyl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ttyl"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn version_prints_the_package_version() {
    for flag in ["--version", "-V"] {
        let output = ttyl(&[flag]);
        assert_eq!(output.status.code(), Some(0), "{flag}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("ttyl {}\n", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn help_prints_usage_and_the_server_variable() {
    for flag in ["--help", "-h"] {
        let output = ttyl(&[flag]);
        assert_eq!(output.status.code(), Some(0), "{flag}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("Usage: ttyl [--version | --help]"),
            "{stdout}"
        );
        assert!(stdout.contains("DEVNULL_URL"), "{stdout}");
        assert!(!stdout.contains("--redirect"), "{stdout}");
    }
}

#[test]
fn unknown_arguments_are_a_usage_error() {
    for args in [
        &["--bogus"][..],
        &["--version", "--bogus"],
        &["--bogus", "x"],
    ] {
        let output = ttyl(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("unknown argument '--bogus'"), "{stderr}");
        assert!(stderr.contains("ttyl --help"), "{stderr}");
    }
}
