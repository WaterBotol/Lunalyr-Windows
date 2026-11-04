//! A minimal, non-validating XML event reader for exported TTML. No DTDs,
//! no external entities: only the five predefined entities and character
//! references are expanded. Malformed documents are rejected, as
//! Foundation's `XMLParser` would.

pub(crate) enum Event<'a> {
    Start { name: &'a str, attributes: Vec<(&'a str, String)> },
    End { name: &'a str },
    Text(String),
}

fn decode_entities(raw: &str) -> Result<String, ()> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let after = &rest[index + 1..];
        let end = after.find(';').ok_or(())?;
        let entity = &after[..end];
        let value = match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).map_err(|_| ())?
                } else if let Some(decimal) = entity.strip_prefix('#') {
                    decimal.parse::<u32>().map_err(|_| ())?
                } else {
                    return Err(());
                };
                char::from_u32(code).filter(|c| *c != '\0').ok_or(())?
            }
        };
        out.push(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn is_name_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '/' | '>' | '=' | '<' | '"' | '\'')
}

/// Reads `input`, calling `handler` for each event. Returns Err for
/// malformed XML or when the handler asks to stop (returns false).
pub(crate) fn read(input: &str, mut handler: impl FnMut(Event) -> bool) -> Result<(), ()> {
    let mut stack: Vec<&str> = Vec::new();
    let mut seen_root = false;
    let mut rest = input.strip_prefix('\u{feff}').unwrap_or(input);
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<?") {
            rest = &after[after.find("?>").ok_or(())? + 2..];
        } else if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->").ok_or(())? + 3..];
        } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>").ok_or(())?;
            if stack.is_empty() {
                return Err(());
            }
            if !handler(Event::Text(after[..end].to_string())) {
                return Err(());
            }
            rest = &after[end + 3..];
        } else if rest.starts_with("<!") {
            // DOCTYPE and other declarations are refused.
            return Err(());
        } else if let Some(after) = rest.strip_prefix("</") {
            let close = after.find('>').ok_or(())?;
            let name = after[..close].trim_end();
            if stack.pop() != Some(name) {
                return Err(());
            }
            if !handler(Event::End { name }) {
                return Err(());
            }
            rest = &after[close + 1..];
        } else if let Some(after) = rest.strip_prefix('<') {
            let name_end = after.find(|c: char| !is_name_char(c)).unwrap_or(after.len());
            let name = &after[..name_end];
            if name.is_empty() || (stack.is_empty() && seen_root) {
                return Err(());
            }
            let mut cursor = &after[name_end..];
            let mut attributes = Vec::new();
            let self_closing;
            loop {
                cursor = cursor.trim_start();
                if let Some(next) = cursor.strip_prefix("/>") {
                    self_closing = true;
                    cursor = next;
                    break;
                }
                if let Some(next) = cursor.strip_prefix('>') {
                    self_closing = false;
                    cursor = next;
                    break;
                }
                let key_end = cursor.find(|c: char| !is_name_char(c)).unwrap_or(cursor.len());
                if key_end == 0 {
                    return Err(());
                }
                let key = &cursor[..key_end];
                cursor = cursor[key_end..].trim_start().strip_prefix('=').ok_or(())?.trim_start();
                let quote = cursor.chars().next().filter(|q| *q == '"' || *q == '\'').ok_or(())?;
                let body = &cursor[1..];
                let close = body.find(quote).ok_or(())?;
                let raw = &body[..close];
                if raw.contains('<') {
                    return Err(());
                }
                let normalized = raw.replace(['\t', '\n', '\r'], " ");
                attributes.push((key, decode_entities(&normalized)?));
                cursor = &body[close + 1..];
            }
            seen_root = true;
            if !handler(Event::Start { name, attributes }) {
                return Err(());
            }
            if self_closing {
                if !handler(Event::End { name }) {
                    return Err(());
                }
            } else {
                stack.push(name);
            }
            rest = cursor;
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let raw = &rest[..end];
            if stack.is_empty() {
                if !raw.trim().is_empty() {
                    return Err(());
                }
            } else {
                let text = decode_entities(&raw.replace("\r\n", "\n"))?;
                if !handler(Event::Text(text)) {
                    return Err(());
                }
            }
            rest = &rest[end..];
        }
    }
    if stack.is_empty() && seen_root {
        Ok(())
    } else {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(xml: &str) -> Result<Vec<String>, ()> {
        let mut out = Vec::new();
        read(xml, |event| {
            out.push(match event {
                Event::Start { name, attributes } => {
                    let attrs: Vec<String> = attributes.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    format!("<{name} {}>", attrs.join(","))
                }
                Event::End { name } => format!("</{name}>"),
                Event::Text(text) => text,
            });
            true
        })?;
        Ok(out)
    }

    #[test]
    fn reads_elements_attributes_text_and_entities() {
        let got =
            events("<?xml version='1.0'?><!-- c --><a x=\"1 &amp; 2\" y='&#x41;'>t &lt;3<b/><![CDATA[<raw>]]></a>")
                .unwrap();
        assert_eq!(got, ["<a x=1 & 2,y=A>", "t <3", "<b >", "</b>", "<raw>", "</a>"]);
    }

    #[test]
    fn rejects_malformed_documents() {
        for xml in [
            "<a><b></a>",
            "<a>",
            "",
            "text",
            "<a>&nbsp;</a>",
            "<a>& </a>",
            "<a></a><b></b>",
            "<a></a>trailing",
            "<!DOCTYPE a><a/>",
            "<a x=1/>",
            "<a x='<'/>",
        ] {
            assert!(events(xml).is_err(), "{xml}");
        }
    }
}
