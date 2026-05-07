#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SourceId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub source: SourceId,
    pub start: usize,
    pub end: usize,
}

impl Span {
    #[must_use]
    pub fn new(source: SourceId, start: usize, end: usize) -> Self {
        Self { source, start, end }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Keyword(String),
    StringLiteral(String),
    Number(String),
    Symbol(char),
    Arrow,
    Newline,
    Eof,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleAst {
    pub source_id: SourceId,
    pub imports: Vec<ImportDecl>,
    pub structs: Vec<StructDecl>,
    pub functions: Vec<FunctionDecl>,
    pub binding_calls: Vec<BindingCall>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportDecl {
    pub module: String,
    pub names: Vec<ImportName>,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportName {
    pub original: String,
    pub alias: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionDecl {
    pub name: String,
    pub owner: Option<String>,
    pub params: Vec<ParamDecl>,
    pub return_type: Option<TypeExpr>,
    pub body: Vec<Stmt>,
    pub body_text: String,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeExpr {
    pub text: String,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Stmt {
    Return {
        expr: Expr,
        span: Span,
    },
    Let {
        name: String,
        explicit_type: Option<TypeExpr>,
        value: Expr,
        span: Span,
    },
    Other {
        text: String,
        span: Span,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Expr {
    pub text: String,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingCall {
    pub kind: BindingKind,
    pub generic: Option<String>,
    pub visible_name: Option<String>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingKind {
    Function,
    PyFunction,
    PyCFunction,
    AddType,
    Method,
    StaticMethod,
    PyMethod,
    PyCMethod,
    PyInit,
    DefaultInit,
}

const BINDING_CALLS: &[(&str, BindingKind)] = &[
    ("def_init_defaultable", BindingKind::DefaultInit),
    ("def_py_c_function", BindingKind::PyCFunction),
    ("def_py_c_method", BindingKind::PyCMethod),
    ("def_py_function", BindingKind::PyFunction),
    ("def_py_method", BindingKind::PyMethod),
    ("def_staticmethod", BindingKind::StaticMethod),
    ("def_py_init", BindingKind::PyInit),
    ("def_function", BindingKind::Function),
    ("def_method", BindingKind::Method),
    ("add_type", BindingKind::AddType),
];

#[must_use]
pub fn lex(source_id: SourceId, source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch == '\n' {
            tokens.push(Token {
                kind: TokenKind::Newline,
                span: Span::new(source_id, start, start + ch.len_utf8()),
            });
            continue;
        }
        if ch.is_whitespace() {
            continue;
        }
        if ch == '#' {
            while let Some((_, next)) = chars.peek() {
                if *next == '\n' {
                    break;
                }
                let _ = chars.next();
            }
            continue;
        }
        if matches!(ch, '"' | '\'') {
            tokens.push(lex_string(source_id, start, ch, &mut chars));
            continue;
        }
        if ch == '-' && chars.peek().is_some_and(|(_, next)| *next == '>') {
            let (_, next) = chars.next().unwrap_or((start, ch));
            tokens.push(Token {
                kind: TokenKind::Arrow,
                span: Span::new(source_id, start, start + ch.len_utf8() + next.len_utf8()),
            });
            continue;
        }
        if ch == '_' || ch.is_ascii_alphabetic() {
            let mut end = start + ch.len_utf8();
            while let Some((index, next)) = chars.peek() {
                if *next == '_' || next.is_ascii_alphanumeric() {
                    end = *index + next.len_utf8();
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            let text = &source[start..end];
            tokens.push(Token {
                kind: if is_keyword(text) {
                    TokenKind::Keyword(text.to_string())
                } else {
                    TokenKind::Identifier(text.to_string())
                },
                span: Span::new(source_id, start, end),
            });
            continue;
        }
        if ch.is_ascii_digit() {
            let mut end = start + ch.len_utf8();
            while let Some((index, next)) = chars.peek() {
                if next.is_ascii_digit() || *next == '.' {
                    end = *index + next.len_utf8();
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token {
                kind: TokenKind::Number(source[start..end].to_string()),
                span: Span::new(source_id, start, end),
            });
            continue;
        }
        tokens.push(Token {
            kind: TokenKind::Symbol(ch),
            span: Span::new(source_id, start, start + ch.len_utf8()),
        });
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(source_id, source.len(), source.len()),
    });
    tokens
}

fn lex_string(
    source_id: SourceId,
    start: usize,
    quote: char,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) -> Token {
    let mut value = String::new();
    let mut escaped = false;
    let mut end = start + quote.len_utf8();
    for (index, ch) in chars.by_ref() {
        end = index + ch.len_utf8();
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            break;
        } else {
            value.push(ch);
        }
    }
    Token {
        kind: TokenKind::StringLiteral(value),
        span: Span::new(source_id, start, end),
    }
}

fn is_keyword(text: &str) -> bool {
    matches!(
        text,
        "alias"
            | "def"
            | "else"
            | "fn"
            | "for"
            | "from"
            | "if"
            | "import"
            | "let"
            | "raises"
            | "return"
            | "struct"
            | "trait"
            | "var"
            | "while"
    )
}

/// Parse the source subset mohaus needs for Python binding analysis.
///
/// # Errors
///
/// Returns an error for malformed import lists, function headers, binding call
/// delimiters, or identifiers that would make exported stub analysis unsafe.
pub fn parse_module(source_id: SourceId, source: &str) -> Result<ModuleAst, String> {
    let lines = source_lines(source);
    let mut diagnostics = Vec::new();
    let imports = parse_imports(source_id, source)?;
    let mut structs = Vec::new();
    let mut functions = Vec::new();
    let mut binding_source = String::new();
    let mut struct_stack: Vec<(usize, String)> = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let line = strip_comment(lines[index].text);
        let trimmed = line.trim();
        let indent = indentation(lines[index].text);
        if !trimmed.is_empty() {
            while struct_stack
                .last()
                .is_some_and(|(struct_indent, _)| indent <= *struct_indent)
            {
                let _ = struct_stack.pop();
            }
        }

        if let Some(name) = parse_struct_name(trimmed) {
            struct_stack.push((indent, name.clone()));
            structs.push(StructDecl {
                name,
                fields: Vec::new(),
                methods: Vec::new(),
                span: Span::new(
                    source_id,
                    lines[index].offset,
                    lines[index].offset + line.len(),
                ),
            });
        }

        if let Some((struct_indent, struct_name)) = struct_stack.last()
            && indent > *struct_indent
            && indent - *struct_indent <= 4
            && let Some(field) = parse_field_decl(source_id, lines[index].offset, trimmed)?
            && let Some(struct_decl) = structs
                .iter_mut()
                .rev()
                .find(|item| item.name == *struct_name)
        {
            struct_decl.fields.push(field);
        }

        if starts_def(trimmed) {
            let (header, consumed, span) = collect_def_header(source_id, &lines, index)?;
            let (body_text, body) = collect_def_body(source_id, &lines, consumed, indent);
            let mut function = parse_def_header(source_id, &header, span)?;
            function.body_text = body_text;
            function.body = body;
            if let Some((_, struct_name)) = struct_stack.last() {
                function.owner = Some(struct_name.clone());
                if let Some(struct_decl) = structs
                    .iter_mut()
                    .rev()
                    .find(|item| item.name == *struct_name)
                {
                    struct_decl.methods.push(function.name.clone());
                }
            }
            functions.push(function);
            index = consumed;
            continue;
        }

        binding_source.push_str(line);
        binding_source.push('\n');
        index += 1;
    }

    let binding_calls = parse_binding_calls(source_id, &binding_source)?;
    diagnostics.extend(validate_lexable(source_id, source));
    Ok(ModuleAst {
        source_id,
        imports,
        structs,
        functions,
        binding_calls,
        diagnostics,
    })
}

fn validate_lexable(source_id: SourceId, source: &str) -> Vec<Diagnostic> {
    lex(source_id, source)
        .into_iter()
        .filter_map(|token| match token.kind {
            TokenKind::Eof => None,
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy)]
struct SourceLine<'a> {
    text: &'a str,
    offset: usize,
}

fn source_lines(source: &str) -> Vec<SourceLine<'_>> {
    let mut out = Vec::new();
    let mut offset = 0;
    for raw in source.split_inclusive('\n') {
        let text = raw.trim_end_matches('\n').trim_end_matches('\r');
        out.push(SourceLine { text, offset });
        offset += raw.len();
    }
    if source.is_empty() || !source.ends_with('\n') {
        out.push(SourceLine {
            text: "",
            offset: source.len(),
        });
    }
    out
}

fn parse_imports(source_id: SourceId, source: &str) -> Result<Vec<ImportDecl>, String> {
    let lines = source_lines(source);
    let mut imports = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = strip_comment(lines[index].text).trim();
        let Some(rest) = trimmed.strip_prefix("from ") else {
            index += 1;
            continue;
        };
        let Some((module, raw_names)) = rest.split_once(" import ") else {
            index += 1;
            continue;
        };

        let start_offset = lines[index].offset;
        let mut names = raw_names.trim().to_string();
        if names.starts_with('(') && matching_delimiter(&names, 0, '(', ')').is_none() {
            let start = index + 1;
            index = start;
            while index < lines.len() {
                names.push(' ');
                names.push_str(strip_comment(lines[index].text).trim());
                if matching_delimiter(&names, 0, '(', ')').is_some() {
                    break;
                }
                index += 1;
            }
            if index == lines.len() {
                return Err(format!(
                    "unterminated import list starting at line {}",
                    start
                ));
            }
        }

        imports.push(ImportDecl {
            module: module.trim().to_string(),
            names: parse_import_names(&names),
            span: Span::new(
                source_id,
                start_offset,
                lines[index].offset + lines[index].text.len(),
            ),
        });
        index += 1;
    }
    Ok(imports)
}

fn parse_import_names(raw_names: &str) -> Vec<ImportName> {
    let names = raw_names
        .trim()
        .strip_prefix('(')
        .unwrap_or(raw_names.trim())
        .trim()
        .strip_suffix(')')
        .unwrap_or_else(|| {
            raw_names
                .trim()
                .strip_prefix('(')
                .unwrap_or(raw_names.trim())
                .trim()
        })
        .trim();
    split_top_level_commas(names)
        .into_iter()
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() || item == "*" {
                return None;
            }
            let parts = item.split_whitespace().collect::<Vec<_>>();
            let original = parts.first()?;
            let alias = if parts.len() >= 3 && parts[1] == "as" {
                parts[2]
            } else {
                original
            };
            (valid_python_identifier(alias) && valid_python_identifier(original)).then(|| {
                ImportName {
                    original: (*original).to_string(),
                    alias: alias.to_string(),
                }
            })
        })
        .collect()
}

fn parse_field_decl(
    source_id: SourceId,
    line_offset: usize,
    trimmed: &str,
) -> Result<Option<FieldDecl>, String> {
    let Some(rest) = trimmed.strip_prefix("var ") else {
        return Ok(None);
    };
    let Some((name_part, ty_part)) = rest.split_once(':') else {
        return Ok(None);
    };
    let Some(name) = name_part.split_whitespace().last() else {
        return Ok(None);
    };
    let ty = strip_default(ty_part).trim();
    if ty.is_empty() {
        return Ok(None);
    }
    Ok(Some(FieldDecl {
        name: clean_identifier(name)?,
        ty: TypeExpr {
            text: normalize_mojo_type(ty),
            span: Span::new(source_id, line_offset, line_offset + trimmed.len()),
        },
        span: Span::new(source_id, line_offset, line_offset + trimmed.len()),
    }))
}

fn parse_struct_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("struct ")?;
    let end = rest
        .find(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
        .unwrap_or(rest.len());
    let name = rest.get(..end)?;
    valid_python_identifier(name).then(|| name.to_string())
}

fn starts_def(trimmed: &str) -> bool {
    trimmed.starts_with("def ") || trimmed.starts_with("def `")
}

fn collect_def_header(
    source_id: SourceId,
    lines: &[SourceLine<'_>],
    start: usize,
) -> Result<(String, usize, Span), String> {
    let mut header = String::new();
    let mut index = start;
    while index < lines.len() {
        if !header.is_empty() {
            header.push(' ');
        }
        header.push_str(strip_comment(lines[index].text).trim());
        if top_level_colon_index(&header).is_some() {
            return Ok((
                header,
                index + 1,
                Span::new(
                    source_id,
                    lines[start].offset,
                    lines[index].offset + lines[index].text.len(),
                ),
            ));
        }
        index += 1;
    }
    Err(format!(
        "unterminated def header starting at line {}",
        start + 1
    ))
}

fn collect_def_body(
    source_id: SourceId,
    lines: &[SourceLine<'_>],
    start: usize,
    def_indent: usize,
) -> (String, Vec<Stmt>) {
    let mut body_text = String::new();
    let mut statements = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index].text;
        let trimmed = strip_comment(line).trim();
        if !trimmed.is_empty() && indentation(line) <= def_indent {
            break;
        }
        body_text.push_str(line);
        body_text.push('\n');
        statements.push(parse_stmt(source_id, lines[index].offset, line));
        index += 1;
    }
    (body_text, statements)
}

fn parse_stmt(source_id: SourceId, line_offset: usize, line: &str) -> Stmt {
    let stripped = strip_comment(line);
    let trimmed = stripped.trim();
    let span = Span::new(source_id, line_offset, line_offset + stripped.len());
    if let Some(expression) = trimmed.strip_prefix("return ") {
        return Stmt::Return {
            expr: Expr {
                text: expression.trim().to_string(),
                span,
            },
            span,
        };
    }
    if let Some(rest) = trimmed
        .strip_prefix("var ")
        .or_else(|| trimmed.strip_prefix("let "))
        && let Some((left, right)) = rest.split_once('=')
    {
        let left = left.trim();
        let name = left.split_once(':').map_or(left, |(name, _)| name).trim();
        if valid_python_identifier(name) {
            let explicit_type = left.split_once(':').map(|(_, ty)| TypeExpr {
                text: normalize_mojo_type(ty),
                span,
            });
            return Stmt::Let {
                name: name.to_string(),
                explicit_type,
                value: Expr {
                    text: right.trim().to_string(),
                    span,
                },
                span,
            };
        }
    }
    Stmt::Other {
        text: trimmed.to_string(),
        span,
    }
}

fn parse_def_header(source_id: SourceId, header: &str, span: Span) -> Result<FunctionDecl, String> {
    let colon =
        top_level_colon_index(header).ok_or_else(|| "def header has no colon".to_string())?;
    let header_without_colon = header[..colon].trim();
    let rest = header_without_colon
        .strip_prefix("def ")
        .ok_or_else(|| format!("not a def header: {header}"))?
        .trim_start();
    let open = find_top_level_char(rest, '(')
        .ok_or_else(|| format!("def header has no parameter list: {header}"))?;
    let raw_name = rest[..open].trim();
    let name = clean_identifier(strip_generic_params(raw_name))?;
    let close = matching_delimiter(rest, open, '(', ')')
        .ok_or_else(|| format!("def `{name}` has an unterminated parameter list"))?;
    let params = parse_params(source_id, &rest[open + 1..close], span)?;
    let suffix = rest[close + 1..].trim();
    let return_type = suffix
        .find("->")
        .map(|arrow| TypeExpr {
            text: suffix[arrow + 2..].trim().to_string(),
            span,
        })
        .filter(|value| !value.text.is_empty());
    Ok(FunctionDecl {
        name,
        owner: None,
        params,
        return_type,
        body: Vec::new(),
        body_text: String::new(),
        span: Span::new(source_id, span.start, span.end),
    })
}

fn parse_params(source_id: SourceId, params: &str, span: Span) -> Result<Vec<ParamDecl>, String> {
    let mut parsed = Vec::new();
    for raw in split_top_level_commas(params) {
        let raw = raw.trim();
        if raw.is_empty() || raw == "*" || raw.starts_with("//") {
            continue;
        }
        let raw = strip_default(raw);
        let Some((name_part, ty_part)) = raw.split_once(':') else {
            continue;
        };
        let Some(name) = name_part.split_whitespace().last() else {
            continue;
        };
        parsed.push(ParamDecl {
            name: clean_identifier(name)?,
            ty: TypeExpr {
                text: ty_part.trim().to_string(),
                span,
            },
            span: Span::new(source_id, span.start, span.end),
        });
    }
    Ok(parsed)
}

fn parse_binding_calls(source_id: SourceId, text: &str) -> Result<Vec<BindingCall>, String> {
    let mut calls = Vec::new();
    for (name, kind) in BINDING_CALLS {
        let mut offset = 0;
        while let Some(found) = text[offset..].find(name) {
            let start = offset + found;
            if !is_call_boundary(text, start, name.len()) {
                offset = start + name.len();
                continue;
            }
            if let Some((call, end)) = parse_binding_call(source_id, text, start, *kind, name)? {
                calls.push((start, call));
                offset = end;
            } else {
                offset = start + name.len();
            }
        }
    }
    calls.sort_by_key(|(position, _)| *position);
    Ok(calls.into_iter().map(|(_, call)| call).collect())
}

fn is_call_boundary(text: &str, start: usize, len: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[start + len..].chars().next();
    let before_ok = before.is_none_or(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()));
    let after_ok = after.is_none_or(|ch| matches!(ch, '[' | '('));
    before_ok && after_ok
}

fn parse_binding_call(
    source_id: SourceId,
    text: &str,
    start: usize,
    kind: BindingKind,
    name: &str,
) -> Result<Option<(BindingCall, usize)>, String> {
    let mut cursor = start + name.len();
    let generic = if text[cursor..].starts_with('[') {
        let close = matching_delimiter(text, cursor, '[', ']')
            .ok_or_else(|| format!("binding `{name}` has unterminated generic args"))?;
        let value = split_top_level_commas(&text[cursor + 1..close])
            .into_iter()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        cursor = close + 1;
        Some(value)
    } else {
        None
    };
    if !text[cursor..].trim_start().starts_with('(') {
        return Ok(None);
    }
    cursor += text[cursor..].find('(').unwrap_or_default();
    let close = matching_delimiter(text, cursor, '(', ')')
        .ok_or_else(|| format!("binding `{name}` has unterminated call args"))?;
    let visible_name = parse_first_string_arg(&text[cursor + 1..close]);
    Ok(Some((
        BindingCall {
            kind,
            generic,
            visible_name,
            span: Span::new(source_id, start, close + 1),
        },
        close + 1,
    )))
}

fn parse_first_string_arg(args: &str) -> Option<String> {
    for arg in split_top_level_commas(args) {
        if let Some(value) = parse_string_literal(arg.trim_start()) {
            return Some(value);
        }
    }
    None
}

#[must_use]
pub fn parse_string_literal(text: &str) -> Option<String> {
    let mut chars = text.char_indices();
    let (_, quote) = chars.next()?;
    if !matches!(quote, '"' | '\'') {
        return None;
    }
    let mut value = String::new();
    let mut escaped = false;
    for (_, ch) in chars {
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return Some(value);
        } else {
            value.push(ch);
        }
    }
    None
}

#[must_use]
pub fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if matches!(ch, '"' | '\'') => quote = Some(ch),
            None if ch == '#' => return &line[..index],
            None => {}
        }
    }
    line
}

#[must_use]
pub fn indentation(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}

#[must_use]
pub fn strip_default(raw: &str) -> &str {
    let mut depth = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => continue,
            None if matches!(ch, '"' | '\'') => {
                quote = Some(ch);
                continue;
            }
            None => {}
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '=' if depth == 0 => return raw[..index].trim_end(),
            _ => {}
        }
    }
    raw
}

#[must_use]
pub fn top_level_colon_index(text: &str) -> Option<usize> {
    find_top_level_char(text, ':')
}

#[must_use]
pub fn find_top_level_char(text: &str, needle: char) -> Option<usize> {
    let mut parens = 0i32;
    let mut brackets = 0i32;
    let mut braces = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => continue,
            None if matches!(ch, '"' | '\'') => {
                quote = Some(ch);
                continue;
            }
            None => {}
        }
        if ch == needle && parens == 0 && brackets == 0 && braces == 0 {
            return Some(index);
        }
        match ch {
            '(' => parens += 1,
            ')' => parens -= 1,
            '[' => brackets += 1,
            ']' => brackets -= 1,
            '{' => braces += 1,
            '}' => braces -= 1,
            _ => {}
        }
    }
    None
}

#[must_use]
pub fn matching_delimiter(text: &str, open: usize, start: char, end: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices().filter(|(index, _)| *index >= open) {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => continue,
            None if matches!(ch, '"' | '\'') => {
                quote = Some(ch);
                continue;
            }
            None => {}
        }
        if ch == start {
            depth += 1;
        } else if ch == end {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

#[must_use]
pub fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut parens = 0i32;
    let mut brackets = 0i32;
    let mut braces = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => continue,
            None if matches!(ch, '"' | '\'') => {
                quote = Some(ch);
                continue;
            }
            None => {}
        }
        match ch {
            '(' => parens += 1,
            ')' => parens -= 1,
            '[' => brackets += 1,
            ']' => brackets -= 1,
            '{' => braces += 1,
            '}' => braces -= 1,
            ',' if parens == 0 && brackets == 0 && braces == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// Normalize ownership modifiers that do not change Python stub types.
#[must_use]
pub fn normalize_mojo_type(ty: &str) -> String {
    ty.trim()
        .trim_start_matches("mut ")
        .trim_start_matches("owned ")
        .trim()
        .to_string()
}

/// Strip a simple top-level generic suffix from an identifier-like callee.
#[must_use]
pub fn strip_generic_params(raw_name: &str) -> &str {
    if let Some(bracket) = find_top_level_char(raw_name, '[') {
        raw_name[..bracket].trim_end()
    } else {
        raw_name
    }
}

/// Convert Mojo/backtick identifiers to Python stub identifiers.
///
/// # Errors
///
/// Returns an error when the identifier cannot be represented in a Python stub.
pub fn clean_identifier(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let name = trimmed
        .strip_prefix('`')
        .and_then(|value| value.strip_suffix('`'))
        .unwrap_or(trimmed);
    if valid_python_identifier(name) {
        Ok(name.to_string())
    } else {
        Err(format!("`{raw}` is not a supported Python identifier"))
    }
}

#[must_use]
pub fn valid_python_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && !is_python_keyword(value)
}

#[must_use]
pub fn is_python_keyword(value: &str) -> bool {
    matches!(
        value,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use crate::{BindingKind, SourceId, TokenKind, lex, parse_module};

    #[test]
    fn lexes_strings_comments_delimiters_and_spans() {
        let tokens = lex(SourceId(7), "def f(x: String): # nope\n  return \"a#b\"\n");
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Keyword("def".to_string()))
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::StringLiteral("a#b".to_string()))
        );
        assert!(
            !tokens
                .iter()
                .any(|token| token.kind == TokenKind::Identifier("nope".to_string()))
        );
        assert_eq!(tokens[0].span.source, SourceId(7));
    }

    #[test]
    fn parses_binding_subset_with_structs_defs_imports_and_returns() {
        let module = parse_module(
            SourceId(0),
            r#"
from create import (
    make_array as make,
)

struct Array:
    var shape: List[Int]

    @staticmethod
    def shape_at(py_self: PythonObject, index: Int) raises -> PythonObject:
        return PythonObject(py_self[].shape[index])

def PyInit__native() -> PythonObject:
    module.add_type[Array]("Array").def_method[Array.shape_at]("shape_at")
"#,
        )
        .unwrap();

        assert_eq!(module.imports[0].names[0].alias, "make");
        assert_eq!(module.structs[0].fields[0].ty.text, "List[Int]");
        assert_eq!(module.functions.len(), 2);
        assert_eq!(module.binding_calls[0].kind, BindingKind::AddType);
        assert_eq!(module.binding_calls[1].kind, BindingKind::Method);
    }

    #[test]
    fn parses_multiline_generic_binding_target() {
        let module = parse_module(
            SourceId(0),
            r#"
def passthrough[T: CollectionElement](value: PythonObject) raises -> PythonObject:
    return value

def PyInit__native() -> PythonObject:
    _ = (
        module
            .def_function[
                passthrough,
            ](
                "passthrough",
            )
    )
"#,
        )
        .unwrap();

        assert_eq!(module.functions[0].name, "passthrough");
        assert_eq!(
            module.binding_calls[0].generic.as_deref(),
            Some("passthrough")
        );
    }
}
