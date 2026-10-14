use std::process::Command;
#[test]
fn mixed_script_locale_and_utf16_order_match_current_js() {
    let labels = vec![
        "a", "A", "Z", "Я", "Ё", "Е", "Ü", "ä", "𐀀", "\u{e000}", "☃", "😀", "a-1", "a_1",
        "[\"a\"]", "[\"Я\"]",
    ];
    let result=Command::new("node").args(["-e","const a=JSON.parse(process.argv[1]); console.log(JSON.stringify({locale:new Intl.Collator().resolvedOptions().locale,collation:[...a].sort((x,y)=>x.localeCompare(y)),lexical:[...a].sort()}))",&serde_json::to_string(&labels).unwrap()]).output().unwrap();
    assert!(result.status.success());
    let result: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        result["locale"], "ru-UA",
        "Source runtime locale changed: reverify port collation"
    );
    let collator = kingmaker_sim::ordering::collator();
    let mut collation = labels.clone();
    collation.sort_by(|a, b| collator.compare(a, b));
    assert_eq!(
        serde_json::to_value(collation).unwrap(),
        result["collation"]
    );
    let mut lexical = labels;
    lexical.sort_by(|a, b| kingmaker_sim::ordering::utf16_compare(a, b));
    assert_eq!(serde_json::to_value(lexical).unwrap(), result["lexical"]);
}
