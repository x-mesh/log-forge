pub(crate) const PATTERN: &str = r#"^(?:<\d+>)?(?:[A-Za-z]{3}\s+\d{1,2} \d{2}:\d{2}:\d{2} \S+ )?haproxy\[\d+\]: (?:\d{1,3}\.){3}\d{1,3}:\d+ \[\d{2}/[A-Za-z]{3}/\d{4}:\d{2}:\d{2}:\d{2}\.\d{3}\] \S+ \S+/\S+ -?\d+/-?\d+/-?\d+/-?\d+/\+?-?\d+ (?:000|[1-5]\d{2}) \d+ \S+ \S+ \S{4} \d+/\d+/\d+/\d+/\+?\d+ \d+/\d+(?: \{[^}]*\}){0,2} "[A-Z]+ [^"\n]+ HTTP/\d(?:\.\d)?"$"#;

pub(crate) fn status(record: &str) -> Option<u16> {
    let mut fields = record.split_whitespace();
    fields.find(|field| field.starts_with("haproxy[") && field.ends_with("]:"))?;
    let status = fields.nth(5)?.parse::<u16>().ok()?;
    (100..=599).contains(&status).then_some(status)
}
