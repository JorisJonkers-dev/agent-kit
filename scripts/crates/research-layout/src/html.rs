//! The text of a fetched page: what `fetch` stores and what every quote is
//! checked against.
//!
//! A page that holds a `<` is read as HTML: comments, `<script>` and
//! `<style>` contents are dropped, a block tag (paragraph, heading, list
//! item, table row, line break, ...) becomes a line break and any other tag a
//! space, and the common entities (`&amp;`, `&lt;`, `&gt;`, `&quot;`,
//! `&apos;`, `&#39;`, `&nbsp;` and numeric ones) are decoded. Every line of
//! the result has its whitespace collapsed and empty lines are dropped, so a
//! quote copied from the stored text matches it byte for byte, and the
//! quote check, which collapses whitespace too, matches across lines.

/// Tags that end a line of text.
const BLOCK_TAGS: [&str; 22] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "li",
    "p",
    "pre",
    "section",
    "tr",
];

/// Tags whose content is never text.
const SKIPPED_TAGS: [&str; 2] = ["script", "style"];

/// The decoded text of one entity at the start of `rest` (which starts with
/// `&`) and its length in bytes, or `None` when it is not one this decoder
/// knows (the `&` is then kept as it stands).
fn entity(rest: &str) -> Option<(String, usize)> {
    let end = rest.char_indices().take(12).find(|(_, c)| *c == ';')?.0;
    let name = &rest[1..end];
    let text = match name {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        "nbsp" => " ".to_string(),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse::<u32>().ok()?,
            };
            char::from_u32(code)?.to_string()
        }
    };
    Some((text, end + 1))
}

/// The lowercase name of the tag that starts at `tag` (the text after `<`,
/// without a leading `/`).
fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('/')
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Collapses every run of whitespace in each line, dropping empty lines;
/// the result ends with a newline unless it is empty.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if !words.is_empty() {
            out.push_str(&words.join(" "));
            out.push('\n');
        }
    }
    out
}

/// The text of `page`: HTML when it holds a `<`, otherwise plain text whose
/// lines are only tidied.
pub fn page_text(page: &str) -> String {
    if !page.contains('<') {
        return tidy(page);
    }
    let lower = page.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while i < page.len() {
        let rest = &page[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").map_or(rest.len(), |p| p + 3);
            out.push(' ');
        } else if rest.starts_with('<') {
            let end = rest.find('>').map_or(rest.len(), |p| p + 1);
            let name = tag_name(&rest[1..end]);
            i += end;
            if SKIPPED_TAGS.contains(&name.as_str()) && !rest.starts_with("</") {
                let close = format!("</{name}");
                i = match lower[i..].find(&close) {
                    Some(p) => {
                        let after = i + p;
                        after
                            + page[after..]
                                .find('>')
                                .map_or(page.len() - after, |q| q + 1)
                    }
                    None => page.len(),
                };
            }
            out.push(if BLOCK_TAGS.contains(&name.as_str()) {
                '\n'
            } else {
                ' '
            });
        } else if let Some((text, len)) = rest.starts_with('&').then(|| entity(rest)).flatten() {
            out.push_str(&text);
            i += len;
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    tidy(&out)
}
