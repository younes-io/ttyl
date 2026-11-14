/// The language a locale or language tag names, by its primary subtag: `fr_FR.UTF-8` and `pt-BR` give `fr`
/// and `pt`. `C` and `POSIX` name none.
pub fn primary(tag: &str) -> Option<String> {
    let subtag = tag.split(['_', '-', '.', '@']).next()?;
    (matches!(subtag.len(), 2 | 3) && subtag.bytes().all(|byte| byte.is_ascii_alphabetic()))
        .then(|| subtag.to_ascii_lowercase())
}

/// The language the device reads in: the first of `LC_ALL`, `LC_MESSAGES` and `LANG`, in that order, that names one.
pub fn device(vars: [Option<&str>; 3]) -> Option<String> {
    vars.into_iter().flatten().find_map(primary)
}

/// The language commits translate into: the one the user picked on dev-null.sh, else the device's, else English.
pub fn target(setting: Option<&str>, device: Option<&str>) -> String {
    (setting.and_then(primary))
        .or_else(|| device.map(str::to_owned))
        .unwrap_or_else(|| "en".to_owned())
}
