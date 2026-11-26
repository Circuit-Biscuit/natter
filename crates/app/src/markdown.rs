//! Discord-flavoured Markdown: **bold**, *italic*, __underline__, ~~strike~~, ||spoiler||, `code`,
//! ```code blocks```, > quotes, # headings, -# subtext, lists, links, <@user> and <#channel> mentions.
//!
//! Parsed into a small tree and rendered as real DOM nodes, never as HTML strings, so message
//! text can never inject markup or script into the app.

use leptos::prelude::*;
use laklak_proto::Id;

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Underline(Vec<Inline>),
    Strike(Vec<Inline>),
    Spoiler(Vec<Inline>),
    Code(String),
    Link { url: String, label: Option<Vec<Inline>> },
    User(Id),
    Channel(Id),
    Everyone(&'static str),
    Br,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Para(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    Sub(Vec<Inline>),
    Code { lang: String, code: String },
    Quote(Vec<Block>),
    List { ordered: bool, start: u32, items: Vec<Vec<Inline>> },
}

// ---------------------------------------------------------------- blocks

pub fn parse(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Para(inline(&para.join("\n"))));
            para.clear();
        }
    };
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("```") {
            // Fenced code: find the closing fence (possibly on the same line).
            let mut body = Vec::new();
            let lang;
            if let Some(end) = rest.find("```") {
                flush(&mut para, &mut blocks);
                blocks.push(Block::Code { lang: String::new(), code: rest[..end].to_string() });
                let after = &rest[end + 3..];
                if !after.trim().is_empty() {
                    para.push(after);
                }
                i += 1;
                continue;
            }
            let head = rest.trim();
            if head.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '#') && !head.is_empty() {
                lang = head.to_string();
            } else {
                lang = String::new();
                if !head.is_empty() {
                    body.push(rest);
                }
            }
            let mut j = i + 1;
            let mut closed = false;
            while j < lines.len() {
                if let Some(pos) = lines[j].find("```") {
                    body.push(&lines[j][..pos]);
                    closed = true;
                    break;
                }
                body.push(lines[j]);
                j += 1;
            }
            if closed {
                flush(&mut para, &mut blocks);
                let mut code = body.join("\n");
                if code.ends_with('\n') {
                    code.pop();
                }
                blocks.push(Block::Code { lang, code });
                i = j + 1;
                continue;
            }
            para.push(line);
            i += 1;
            continue;
        }
        if let Some(rest) = t.strip_prefix(">>> ") {
            flush(&mut para, &mut blocks);
            let mut rest_lines = vec![rest];
            rest_lines.extend(&lines[i + 1..]);
            blocks.push(Block::Quote(parse(&rest_lines.join("\n"))));
            break;
        }
        if t.starts_with("> ") || t == ">" {
            flush(&mut para, &mut blocks);
            let mut q = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim_start();
                if let Some(r) = l.strip_prefix("> ") {
                    q.push(r);
                } else if l == ">" {
                    q.push("");
                } else {
                    break;
                }
                i += 1;
            }
            blocks.push(Block::Quote(parse(&q.join("\n"))));
            continue;
        }
        if let Some((level, rest)) = heading(t) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(level, inline(rest)));
            i += 1;
            continue;
        }
        if let Some(rest) = t.strip_prefix("-# ") {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Sub(inline(rest)));
            i += 1;
            continue;
        }
        if list_item(line).is_some() {
            flush(&mut para, &mut blocks);
            let (ordered, start, _) = list_item(line).unwrap();
            let mut items = Vec::new();
            while i < lines.len() {
                match list_item(lines[i]) {
                    Some((o, _, text)) if o == ordered => items.push(inline(text)),
                    _ => break,
                }
                i += 1;
            }
            blocks.push(Block::List { ordered, start, items });
            continue;
        }
        para.push(line);
        i += 1;
    }
    flush(&mut para, &mut blocks);
    blocks
}

fn heading(t: &str) -> Option<(u8, &str)> {
    for (level, p) in [(3u8, "### "), (2, "## "), (1, "# ")] {
        if let Some(r) = t.strip_prefix(p) {
            return Some((level, r));
        }
    }
    None
}

fn list_item(line: &str) -> Option<(bool, u32, &str)> {
    let t = line.trim_start();
    if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        return Some((false, 1, r));
    }
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() && digits.len() < 4 {
        if let Some(r) = t[digits.len()..].strip_prefix(". ") {
            return Some((true, digits.parse().unwrap_or(1), r));
        }
    }
    None
}

// ---------------------------------------------------------------- inline

pub fn inline(s: &str) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut text = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    let push_text = |text: &mut String, out: &mut Vec<Inline>| {
        if !text.is_empty() {
            out.push(Inline::Text(std::mem::take(text)));
        }
    };
    while i < s.len() {
        let rest = &s[i..];
        // Escapes.
        if b[i] == b'\\' && i + 1 < s.len() {
            let next = rest[1..].chars().next().unwrap();
            if "\\*_~`|<>[]()#-@:".contains(next) {
                text.push(next);
                i += 1 + next.len_utf8();
                continue;
            }
        }
        if b[i] == b'\n' {
            push_text(&mut text, &mut out);
            out.push(Inline::Br);
            i += 1;
            continue;
        }
        // Code spans: `x` or ``x``.
        if b[i] == b'`' {
            let ticks = if rest.starts_with("``") { 2 } else { 1 };
            let delim = &rest[..ticks];
            if let Some(end) = rest[ticks..].find(delim) {
                if end > 0 {
                    push_text(&mut text, &mut out);
                    out.push(Inline::Code(rest[ticks..ticks + end].to_string()));
                    i += ticks * 2 + end;
                    continue;
                }
            }
        }
        // Paired delimiters, longest first.
        let mut matched = false;
        for (delim, kind) in [("***", 0u8), ("||", 1), ("**", 2), ("__", 3), ("~~", 4), ("*", 5), ("_", 6)] {
            if !rest.starts_with(delim) {
                continue;
            }
            if delim == "_" && s[..i].chars().last().is_some_and(|c| c.is_alphanumeric()) {
                break;
            }
            if let Some(end) = find_close(&rest[delim.len()..], delim) {
                let inner = &rest[delim.len()..delim.len() + end];
                let after = &rest[delim.len() * 2 + end..];
                if delim == "_" && after.chars().next().is_some_and(|c| c.is_alphanumeric()) {
                    continue;
                }
                if inner.trim().is_empty() {
                    continue;
                }
                push_text(&mut text, &mut out);
                let kids = inline(inner);
                out.push(match kind {
                    0 => Inline::Bold(vec![Inline::Italic(kids)]),
                    1 => Inline::Spoiler(kids),
                    2 => Inline::Bold(kids),
                    3 => Inline::Underline(kids),
                    4 => Inline::Strike(kids),
                    _ => Inline::Italic(kids),
                });
                i += delim.len() * 2 + end;
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }
        // Masked links: [label](https://...)
        if b[i] == b'[' {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close + 2..].find(')') {
                    let url = &rest[close + 2..close + 2 + end];
                    let label = &rest[1..close];
                    if is_url(url) && !label.is_empty() && !label.contains('\n') {
                        push_text(&mut text, &mut out);
                        out.push(Inline::Link { url: url.to_string(), label: Some(inline(label)) });
                        i += close + 3 + end;
                        continue;
                    }
                }
            }
        }
        // Mentions: <@123>, <#123>
        if rest.starts_with("<@") || rest.starts_with("<#") {
            if let Some(end) = rest.find('>') {
                if let Ok(id) = rest[2..end].parse::<Id>() {
                    push_text(&mut text, &mut out);
                    out.push(if rest.starts_with("<@") { Inline::User(id) } else { Inline::Channel(id) });
                    i += end + 1;
                    continue;
                }
            }
        }
        if rest.starts_with("@everyone") || rest.starts_with("@here") {
            let word = if rest.starts_with("@everyone") { "@everyone" } else { "@here" };
            push_text(&mut text, &mut out);
            out.push(Inline::Everyone(word));
            i += word.len();
            continue;
        }
        // <https://…>: a link without a preview (the server skips it); the brackets don't show.
        if (rest.starts_with("<https://") || rest.starts_with("<http://")) && let Some(end) = rest.find('>') {
            let url = &rest[1..end];
            if !url.contains(char::is_whitespace) {
                push_text(&mut text, &mut out);
                out.push(Inline::Link { url: url.to_string(), label: None });
                i += end + 1;
                continue;
            }
        }
        // Bare links.
        if rest.starts_with("https://") || rest.starts_with("http://") {
            let end = rest.find(|c: char| c.is_whitespace() || c == '<' || c == '>').unwrap_or(rest.len());
            let mut url = &rest[..end];
            // Trailing punctuation belongs to the sentence, not the link.
            while url.ends_with(['.', ',', ')', '!', '?', ':', ';', '"', '\'']) && !balanced_paren(url) {
                url = &url[..url.len() - 1];
            }
            if url.len() > 8 {
                push_text(&mut text, &mut out);
                out.push(Inline::Link { url: url.to_string(), label: None });
                i += url.len();
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        text.push(c);
        i += c.len_utf8();
    }
    push_text(&mut text, &mut out);
    out
}

fn balanced_paren(url: &str) -> bool {
    url.ends_with(')') && url.matches('(').count() >= url.matches(')').count()
}

fn find_close(s: &str, delim: &str) -> Option<usize> {
    let mut i = 0;
    while let Some(p) = s[i..].find(delim) {
        let at = i + p;
        if at > 0 && s.as_bytes()[at - 1] == b'\\' {
            i = at + delim.len();
            continue;
        }
        // A `**` inside `*italic*` is bold, not the closing `*`.
        if delim.len() == 1 && s[at + 1..].starts_with(delim) {
            i = at + 2;
            continue;
        }
        return Some(at);
    }
    None
}

pub fn is_url(s: &str) -> bool {
    (s.starts_with("https://") || s.starts_with("http://")) && !s.contains(char::is_whitespace) && s.len() > 8
}

/// Plain text for notifications and reply previews: mentions resolved, markup stripped.
pub fn plain(text: &str, name: &dyn Fn(Id) -> String) -> String {
    fn walk(v: &[Inline], name: &dyn Fn(Id) -> String, out: &mut String) {
        for i in v {
            match i {
                Inline::Text(t) | Inline::Code(t) => out.push_str(t),
                Inline::Bold(k) | Inline::Italic(k) | Inline::Underline(k) | Inline::Strike(k) => walk(k, name, out),
                Inline::Spoiler(_) => out.push_str("▒▒▒▒"),
                Inline::Link { url, label } => match label {
                    Some(l) => walk(l, name, out),
                    None => out.push_str(url),
                },
                Inline::User(id) => {
                    out.push('@');
                    out.push_str(&name(*id));
                }
                Inline::Channel(_) => out.push_str("#channel"),
                Inline::Everyone(w) => out.push_str(w),
                Inline::Br => out.push(' '),
            }
        }
    }
    let mut out = String::new();
    for b in parse(text) {
        match b {
            Block::Para(v) | Block::Heading(_, v) | Block::Sub(v) => walk(&v, name, &mut out),
            Block::Code { code, .. } => out.push_str(&code),
            Block::Quote(inner) => {
                for ib in inner {
                    if let Block::Para(v) = ib {
                        walk(&v, name, &mut out);
                    }
                }
            }
            Block::List { items, .. } => {
                for it in items {
                    walk(&it, name, &mut out);
                    out.push(' ');
                }
            }
        }
        out.push(' ');
    }
    out.trim().to_string()
}

// ---------------------------------------------------------------- rendering

pub fn render(text: &str, space: Option<Id>) -> AnyView {
    // Bots and API clients may send `:shortcodes:`; show them as emoji like the composer does.
    let blocks = parse(&crate::util::expand_shortcodes(text));
    view! { <div class="md">{blocks.into_iter().map(|b| block(b, space)).collect_view()}</div> }.into_any()
}

fn block(b: Block, space: Option<Id>) -> AnyView {
    match b {
        Block::Para(v) => view! { <p>{inlines(v, space)}</p> }.into_any(),
        Block::Heading(1, v) => view! { <h1>{inlines(v, space)}</h1> }.into_any(),
        Block::Heading(2, v) => view! { <h2>{inlines(v, space)}</h2> }.into_any(),
        Block::Heading(_, v) => view! { <h3>{inlines(v, space)}</h3> }.into_any(),
        Block::Sub(v) => view! { <p class="subtext">{inlines(v, space)}</p> }.into_any(),
        Block::Code { lang, code } => {
            let copy = code.clone();
            let copied = RwSignal::new(false);
            view! {
                <div class="codeblock">
                    <pre><code data-lang=lang>{code}</code></pre>
                    <button class="copy" on:click=move |_| {
                        crate::util::copy_text(&copy);
                        copied.set(true);
                        set_timeout(move || copied.set(false), std::time::Duration::from_millis(1500));
                    }>{move || if copied.get() { "Copied" } else { "Copy" }}</button>
                </div>
            }
            .into_any()
        }
        Block::Quote(inner) => {
            view! { <blockquote>{inner.into_iter().map(|b| block(b, space)).collect_view()}</blockquote> }.into_any()
        }
        Block::List { ordered: true, start, items } => view! {
            <ol start=start>{items.into_iter().map(|it| view! { <li>{inlines(it, space)}</li> }).collect_view()}</ol>
        }
        .into_any(),
        Block::List { items, .. } => view! {
            <ul>{items.into_iter().map(|it| view! { <li>{inlines(it, space)}</li> }).collect_view()}</ul>
        }
        .into_any(),
    }
}

fn inlines(v: Vec<Inline>, space: Option<Id>) -> AnyView {
    v.into_iter().map(|i| one(i, space)).collect_view().into_any()
}

fn one(i: Inline, space: Option<Id>) -> AnyView {
    match i {
        Inline::Text(t) => t.into_any(),
        Inline::Br => view! { <br/> }.into_any(),
        Inline::Bold(k) => view! { <strong>{inlines(k, space)}</strong> }.into_any(),
        Inline::Italic(k) => view! { <em>{inlines(k, space)}</em> }.into_any(),
        Inline::Underline(k) => view! { <u>{inlines(k, space)}</u> }.into_any(),
        Inline::Strike(k) => view! { <s>{inlines(k, space)}</s> }.into_any(),
        Inline::Code(c) => view! { <code class="inline">{c}</code> }.into_any(),
        Inline::Spoiler(k) => {
            let shown = RwSignal::new(false);
            view! {
                <span class="spoiler" class:shown=move || shown.get() on:click=move |e| {
                    e.stop_propagation();
                    shown.set(true);
                }>{inlines(k, space)}</span>
            }
            .into_any()
        }
        Inline::Link { url, label } => {
            // A link to one of our channels shows as the channel, like a <#channel> mention.
            let s = crate::store::store();
            if label.is_none() && let Some(ch) = crate::util::channel_link(&url).filter(|&ch| s.channel(ch).is_some()) {
                return one(Inline::Channel(ch), space);
            }
            let href = url.clone();
            let title = url.clone();
            let body = match label {
                Some(l) => inlines(l, space),
                None => url.into_any(),
            };
            view! {
                <a class="link" href=href.clone() title=title on:click=move |e| {
                    e.prevent_default();
                    crate::desktop::open_url(&href);
                }>{body}</a>
            }
            .into_any()
        }
        Inline::User(id) => {
            let s = crate::store::store();
            let me = s.me_id();
            view! {
                <span class="mention" class:me=id == me on:click=move |e| s.open_profile(&e, id)>
                    "@" {move || s.name(id, space)}
                </span>
            }
            .into_any()
        }
        Inline::Channel(id) => {
            let s = crate::store::store();
            view! {
                <span class="mention" on:click=move |_| crate::store::open_channel(id)>
                    {move || match s.channel(id) {
                        Some(c) if c.kind == laklak_proto::ChannelKind::Voice => format!("🔊 {}", c.name),
                        Some(c) => format!("#{}", c.name),
                        None => "#unknown".into(),
                    }}
                </span>
            }
            .into_any()
        }
        Inline::Everyone(w) => view! { <span class="mention">{w}</span> }.into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_basics() {
        assert_eq!(inline("a **b** c"), vec![Inline::Text("a ".into()), Inline::Bold(vec![Inline::Text("b".into())]), Inline::Text(" c".into())]);
        assert_eq!(inline("snake_case_word"), vec![Inline::Text("snake_case_word".into())]);
        assert_eq!(inline("see <https://x.org/a>"), vec![Inline::Text("see ".into()), Inline::Link { url: "https://x.org/a".into(), label: None }]);
        assert_eq!(inline("`**x**`"), vec![Inline::Code("**x**".into())]);
        assert_eq!(inline("hi <@42>!")[1], Inline::User(42));
        assert_eq!(inline("see https://x.org/a."), vec![Inline::Text("see ".into()), Inline::Link { url: "https://x.org/a".into(), label: None }, Inline::Text(".".into())]);
        assert_eq!(inline("\\*not\\*"), vec![Inline::Text("*not*".into())]);
        assert!(matches!(inline("||boo||")[0], Inline::Spoiler(_)));
        assert!(matches!(inline("[site](https://a.bc)")[0], Inline::Link { label: Some(_), .. }));
        assert!(matches!(inline("[x](javascript:alert(1))")[0], Inline::Text(_)));
    }

    #[test]
    fn blocks() {
        let b = parse("# Title\n> quoted\nplain\n```rs\nfn x() {}\n```\n- a\n- b");
        assert!(matches!(b[0], Block::Heading(1, _)));
        assert!(matches!(b[1], Block::Quote(_)));
        assert!(matches!(b[2], Block::Para(_)));
        assert_eq!(b[3], Block::Code { lang: "rs".into(), code: "fn x() {}".into() });
        assert!(matches!(&b[4], Block::List { items, .. } if items.len() == 2));
        assert_eq!(plain("**hi** <@1>", &|_| "Alice".into()), "hi @Alice");
    }
}
