use bande_xdg::Locale;

pub fn best<'a>(
    present: impl Iterator<Item = Option<&'a str>>,
    locale: Option<&Locale>,
) -> Option<String> {
    let present: Vec<_> = present.collect();
    let wanted = locale.map(Locale::keys).unwrap_or_default();
    wanted
        .into_iter()
        .find(|key| present.contains(&Some(key.as_str())))
        .or_else(|| {
            if present.contains(&None) {
                None
            } else if present.contains(&Some("en")) {
                Some("en".to_owned())
            } else {
                present.first().copied().flatten().map(str::to_owned)
            }
        })
}
