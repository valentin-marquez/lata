//! Workshop descriptions are BBCode. This reads them as plain text (a result's summary) or as
//! HTML for the mod's page, which gpui-component's text view renders.

/// One piece of a description: text, or a tag.
#[derive(Debug, PartialEq)]
enum Token<'a> {
    Text(&'a str),
    /// `[name]` or `[name=arg]`, name in lowercase.
    Open {
        name: String,
        arg: Option<&'a str>,
        raw: &'a str,
    },
    /// `[/name]`.
    Close {
        name: String,
        raw: &'a str,
    },
}

/// A description cut into text and tags. A tag is a short word (`b`, `/url`, `url=...`, `*`) on
/// one line; any other bracket is part of the text, like the `[B42.21+ MP]` many titles start
/// with.
fn tokens(bbcode: &str) -> Vec<Token<'_>> {
    let mut found = Vec::new();
    let mut rest = bbcode;
    let mut text_from = 0;
    let mut at = 0;
    while let Some(open) = rest.find('[') {
        let after = &rest[open..];
        let tag = after.find(']').and_then(|close| {
            let inner = &after[1..close];
            let (closing, inner_name) = match inner.strip_prefix('/') {
                Some(name) => (true, name),
                None => (false, inner),
            };
            let (name, arg) = match inner_name.split_once('=') {
                Some((name, arg)) => (name, Some(arg.trim_matches(['"', '\'']))),
                None => (inner_name, None),
            };
            let name = name.trim();
            let ok = !inner.contains('\n')
                && (name == "*"
                    || (!name.is_empty()
                        && name.len() <= 10
                        && name.bytes().all(|b| b.is_ascii_alphanumeric())));
            ok.then(|| {
                let raw = &after[..=close];
                let name = name.to_ascii_lowercase();
                let token = match closing {
                    true => Token::Close { name, raw },
                    false => Token::Open { name, arg, raw },
                };
                (token, close + 1)
            })
        });
        match tag {
            Some((token, len)) => {
                let start = at + open;
                if start > text_from {
                    found.push(Token::Text(&bbcode[text_from..start]));
                }
                found.push(token);
                at = start + len;
                text_from = at;
                rest = &bbcode[at..];
            }
            None => {
                at += open + 1;
                rest = &bbcode[at..];
            }
        }
    }
    if text_from < bbcode.len() {
        found.push(Token::Text(&bbcode[text_from..]));
    }
    found
}

/// The start of a Workshop description as plain text: BBCode tags and bare links left out,
/// lines joined.
pub fn plain(bbcode: &str) -> String {
    let mut text = String::with_capacity(bbcode.len());
    for token in tokens(bbcode) {
        match token {
            Token::Text(t) => text.push_str(t),
            _ => text.push(' '),
        }
    }
    text.split_whitespace()
        .filter(|word| !word.starts_with("http://") && !word.starts_with("https://"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The pictures a description shows (`[img]`), in order; only https ones.
pub fn images(bbcode: &str) -> Vec<String> {
    let tokens = tokens(bbcode);
    let mut found = Vec::new();
    for (ix, token) in tokens.iter().enumerate() {
        if let Token::Open { name, .. } = token
            && name == "img"
            && let Some(Token::Text(src)) = tokens.get(ix + 1)
        {
            let src = src.trim();
            if src.starts_with("https://") && !found.iter().any(|s| s == src) {
                found.push(src.to_owned());
            }
        }
    }
    found
}

/// Where a link goes, or `None` for one that isn't a web address. Authors often leave the
/// scheme out (`patreon.com/name`).
fn link(href: &str) -> Option<String> {
    let href = href.trim();
    if href.starts_with("https://") || href.starts_with("http://") {
        return Some(href.to_owned());
    }
    let host = href.split('/').next()?;
    (host.contains('.') && !href.contains(char::is_whitespace)).then(|| format!("https://{href}"))
}

fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// A description as HTML: headings one level down (the page's title is the top one), lists,
/// links, pictures, quotes, code and tables. Tags it doesn't know stay as text.
pub fn html(bbcode: &str) -> String {
    let tokens = tokens(bbcode);
    let mut out = String::with_capacity(bbcode.len() * 2);
    // Inside a list or table, line breaks are the source's layout, not the text's.
    let mut blocks = 0usize;
    let mut code = false;
    // A block tag ends its line: the line break after it isn't a blank line.
    let mut after_block = false;
    // For each open list, whether an item is open in it.
    let mut items: Vec<bool> = Vec::new();
    let mut links = 0usize;
    let mut ix = 0;
    while ix < tokens.len() {
        match &tokens[ix] {
            Token::Text(text) => {
                let mut text = *text;
                if after_block {
                    text = text
                        .strip_prefix("\r\n")
                        .or(text.strip_prefix('\n'))
                        .unwrap_or(text);
                }
                for (n, line) in text.split('\n').enumerate() {
                    if n > 0 && (blocks == 0 || code) {
                        out.push_str("<br>");
                    }
                    escape(line.trim_end_matches('\r'), &mut out);
                }
                after_block = false;
            }
            Token::Open { name, arg, raw } => {
                let mut block = false;
                match name.as_str() {
                    "b" => out.push_str("<strong>"),
                    "i" => out.push_str("<em>"),
                    "u" => out.push_str("<u>"),
                    "strike" | "s" => out.push_str("<del>"),
                    "spoiler" => out.push_str("<mark>"),
                    "h1" | "h2" | "h3" => {
                        let level = name[1..].parse::<u8>().unwrap_or(1) + 1;
                        out.push_str(&format!("<h{level}>"));
                    }
                    "quote" => {
                        out.push_str("<blockquote>");
                        block = true;
                    }
                    "code" => {
                        out.push_str("<code>");
                        code = true;
                        block = true;
                    }
                    "list" | "olist" => {
                        out.push_str(if name == "list" { "<ul>" } else { "<ol>" });
                        items.push(false);
                        blocks += 1;
                        block = true;
                    }
                    "*" if !items.is_empty() => {
                        if items.last() == Some(&true) {
                            out.push_str("</li>");
                        }
                        out.push_str("<li>");
                        if let Some(open) = items.last_mut() {
                            *open = true;
                        }
                    }
                    "table" => {
                        out.push_str("<table>");
                        blocks += 1;
                        block = true;
                    }
                    "tr" | "th" | "td" if blocks > 0 => out.push_str(&format!("<{name}>")),
                    "url" => {
                        // `[url]address[/url]` shows the address; `[url=address]` its text.
                        let (href, shown) = match arg {
                            Some(href) => (Some(*href), None),
                            None => match tokens.get(ix + 1) {
                                Some(Token::Text(t)) => (Some(t.trim()), Some(*t)),
                                _ => (None, None),
                            },
                        };
                        if let Some(href) = href.and_then(link) {
                            out.push_str("<a href=\"");
                            escape(&href, &mut out);
                            out.push_str("\">");
                            links += 1;
                            if let Some(shown) = shown {
                                escape(shown, &mut out);
                                ix += 1;
                            }
                        }
                    }
                    "img" => {
                        if let Some(Token::Text(src)) = tokens.get(ix + 1) {
                            let src = src.trim();
                            if src.starts_with("https://") {
                                out.push_str("<img src=\"");
                                escape(src, &mut out);
                                out.push_str("\">");
                            }
                            ix += 1;
                        }
                        // Its closing tag is skipped with it.
                        if matches!(tokens.get(ix + 1), Some(Token::Close { name, .. }) if name == "img")
                        {
                            ix += 1;
                        }
                    }
                    "noparse" => {
                        // Everything up to its end is text, tags included.
                        ix += 1;
                        while let Some(token) = tokens.get(ix) {
                            match token {
                                Token::Close { name, .. } if name == "noparse" => break,
                                Token::Text(t) => escape(t, &mut out),
                                Token::Open { raw, .. } | Token::Close { raw, .. } => {
                                    escape(raw, &mut out)
                                }
                            }
                            ix += 1;
                        }
                    }
                    "hr" => {
                        out.push_str("<br>");
                        block = true;
                    }
                    _ => escape(raw, &mut out),
                }
                after_block = block;
            }
            Token::Close { name, raw } => {
                let mut block = true;
                match name.as_str() {
                    "b" => out.push_str("</strong>"),
                    "i" => out.push_str("</em>"),
                    "u" => out.push_str("</u>"),
                    "strike" | "s" => out.push_str("</del>"),
                    "spoiler" => out.push_str("</mark>"),
                    "h1" | "h2" | "h3" => {
                        let level = name[1..].parse::<u8>().unwrap_or(1) + 1;
                        out.push_str(&format!("</h{level}>"));
                    }
                    "quote" => out.push_str("</blockquote>"),
                    "code" => {
                        out.push_str("</code>");
                        code = false;
                    }
                    "list" | "olist" if !items.is_empty() => {
                        if items.pop() == Some(true) {
                            out.push_str("</li>");
                        }
                        out.push_str(if name == "list" { "</ul>" } else { "</ol>" });
                        blocks = blocks.saturating_sub(1);
                    }
                    "table" if blocks > 0 => {
                        out.push_str("</table>");
                        blocks -= 1;
                    }
                    "tr" | "th" | "td" if blocks > 0 => out.push_str(&format!("</{name}>")),
                    "url" if links > 0 => {
                        out.push_str("</a>");
                        links -= 1;
                        block = false;
                    }
                    "url" | "hr" | "noparse" => block = false,
                    _ => {
                        escape(raw, &mut out);
                        block = false;
                    }
                }
                after_block = block
                    && matches!(
                        name.as_str(),
                        "h1" | "h2" | "h3" | "quote" | "code" | "list" | "olist" | "table" | "tr"
                    );
            }
        }
        ix += 1;
    }
    // Authors space sections with runs of blank lines; two breaks are one blank line.
    while out.contains("<br><br><br>") {
        out = out.replace("<br><br><br>", "<br><br>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{html, images, plain};

    #[test]
    fn a_description_reads_as_plain_text() {
        // The start of 3780639614's, as QueryFiles returns it.
        assert_eq!(
            plain("[h1]Push Vehicle[/h1]\nEver had a [b]vehicle[/b] blocking"),
            "Push Vehicle Ever had a vehicle blocking"
        );
        assert_eq!(
            plain("https://i.imgur.com/8CRHL5v.png B41 and [url=https://x.y/z]B42[/url]"),
            "B41 and B42"
        );
        // Brackets that aren't BBCode stay.
        assert_eq!(plain("[B42.21+ MP] Push"), "[B42.21+ MP] Push");
        assert_eq!(
            plain("Ages [5 to 10\n] and [ ] a[b"),
            "Ages [5 to 10 ] and [ ] a[b"
        );
        assert_eq!(plain(""), "");
    }

    #[test]
    fn a_description_reads_as_html() {
        // Pieces of 3837899636's and 2392709985's descriptions.
        assert_eq!(
            html("[h1]Project Zomboid 42.22[/h1]\nI made this mod.\nWhat a thrill!"),
            "<h2>Project Zomboid 42.22</h2>I made this mod.<br>What a thrill!"
        );
        assert_eq!(
            html("[list]\n[*] [url=https://x.y/1]SMB Steps[/url] (optional).\n[*] two\n[/list]\n\nNext"),
            "<ul><li> <a href=\"https://x.y/1\">SMB Steps</a> (optional).<li> two</li></ul><br>Next"
                .replace("<li> two", "</li><li> two")
        );
        assert_eq!(
            html("[url=patreon.com/_tsar][img]https://a.akamaihd.net/ugc/1/C7/[/img][/url]"),
            "<a href=\"https://patreon.com/_tsar\"><img src=\"https://a.akamaihd.net/ugc/1/C7/\"></a>"
        );
        assert_eq!(
            html("[url]https://x.y/z[/url] [strike]Hideo[/strike] a < b"),
            "<a href=\"https://x.y/z\">https://x.y/z</a> <del>Hideo</del> a &lt; b"
        );
        // Code keeps its lines; unknown tags and plain brackets stay as text.
        assert_eq!(
            html("[code]\nlocal a = 1\nend[/code][B42] [foo]x[/foo]"),
            "<code>local a = 1<br>end</code>[B42] [foo]x[/foo]"
        );
        assert_eq!(html("a\n\n\n\n\nb"), "a<br><br>b");
        assert_eq!(html("[url=javascript:x]y[/url]"), "y");
    }

    #[test]
    fn finds_the_pictures_of_a_description() {
        assert_eq!(
            images(
                "[img]https://a.b/1.png[/img] [img]http://a.b/2.png[/img][img] https://a.b/1.png [/img]"
            ),
            vec!["https://a.b/1.png".to_owned()]
        );
    }
}
