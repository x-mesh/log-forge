#[derive(Debug, PartialEq)]
pub(crate) enum Part<'a> {
    Text(&'a str),
    Field(&'a str),
}

pub(crate) struct Parts<'a> {
    remaining: &'a str,
    failed: bool,
}
pub(crate) fn parse(template: &str) -> Parts<'_> {
    Parts {
        remaining: template,
        failed: false,
    }
}
impl<'a> Iterator for Parts<'a> {
    type Item = Result<Part<'a>, String>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.remaining.is_empty() {
            return None;
        }
        let position = self
            .remaining
            .find(['{', '}'])
            .unwrap_or(self.remaining.len());
        if position > 0 {
            let text = &self.remaining[..position];
            self.remaining = &self.remaining[position..];
            return Some(Ok(Part::Text(text)));
        }
        if self.remaining.starts_with("{{") {
            self.remaining = &self.remaining[2..];
            return Some(Ok(Part::Text("{")));
        }
        if self.remaining.starts_with("}}") {
            self.remaining = &self.remaining[2..];
            return Some(Ok(Part::Text("}")));
        }
        if self.remaining.starts_with('}') {
            self.failed = true;
            return Some(Err("unmatched closing brace in custom template".into()));
        }
        let rest = &self.remaining[1..];
        let Some(end) = rest.find('}') else {
            self.failed = true;
            return Some(Err("unclosed custom template field".into()));
        };
        let spec = &rest[..end];
        let name = spec.split(':').next().unwrap_or("");
        if spec.contains('{') || name.is_empty() || name.chars().any(char::is_whitespace) {
            self.failed = true;
            return Some(Err("invalid custom template field".into()));
        }
        self.remaining = &rest[end + 1..];
        Some(Ok(Part::Field(spec)))
    }
}
