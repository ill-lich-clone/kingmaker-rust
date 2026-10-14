//! Current Node v24.15 / ICU78.2 runtime uses ru-UA for localeCompare.
use icu_collator::{Collator, CollatorBorrowed};
pub fn collator() -> CollatorBorrowed<'static> {
    let locale: icu_locale_core::Locale = "ru-UA".parse().expect("constant locale");
    Collator::try_new(locale.into(), Default::default()).expect("bundled ICU ru-UA collation")
}
pub fn utf16_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
