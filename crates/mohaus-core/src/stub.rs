use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{MojoModule, ProjectConfig};
use crate::error::{MohausError, Result};
use crate::wheel::write_file;

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

/// A generated Python stub for one configured Mojo extension module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleStub {
    pub path: PathBuf,
    pub text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct StubBindings {
    functions: BTreeMap<String, StubFunction>,
    classes: BTreeMap<String, StubClass>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StubFunction {
    params: Vec<StubParam>,
    returns: String,
    varargs: bool,
}

impl StubFunction {
    fn varargs(returns: impl Into<String>) -> Self {
        Self {
            params: Vec::new(),
            returns: returns.into(),
            varargs: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StubParam {
    name: String,
    annotation: String,
    keyword_rest: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct StubClass {
    init: Option<StubFunction>,
    methods: BTreeMap<String, StubFunction>,
    static_methods: BTreeMap<String, StubFunction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedSource {
    defs: BTreeMap<String, MojoDef>,
    field_types: BTreeMap<String, String>,
    imports: BTreeMap<String, String>,
    calls: Vec<BindingCall>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StubSource {
    module_name: Option<String>,
    text: String,
    is_entry: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MojoDef {
    params: Vec<MojoParam>,
    return_type: Option<String>,
    body: String,
    owner: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MojoParam {
    name: String,
    ty: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BindingCall {
    kind: BindingKind,
    generic: Option<String>,
    visible_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BindingKind {
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

/// Return the `.pyi` output path for a module compiled to `extension_output`.
///
/// # Errors
///
/// Returns an error when the extension output path has no parent directory.
pub fn module_stub_path_for_extension(
    module: &MojoModule,
    extension_output: &Path,
) -> Result<PathBuf> {
    let Some(output_dir) = extension_output.parent() else {
        return Err(MohausError::InvalidProject {
            message: format!(
                "extension output has no parent directory: {}",
                extension_output.display()
            ),
        });
    };
    Ok(module_stub_path_in_dir(module, output_dir))
}

/// Build the generated stub path and contents for one configured module.
///
/// # Errors
///
/// Returns an error when the module entry source cannot be read or parsed, or
/// the extension output path has no parent directory.
pub fn module_stub_plan_for_extension(
    config: &ProjectConfig,
    module: &MojoModule,
    extension_output: &Path,
) -> Result<ModuleStub> {
    let path = module_stub_path_for_extension(module, extension_output)?;
    let text = render_module_stub(config, module)?;
    Ok(ModuleStub { path, text })
}

/// Write the generated `.pyi` next to a compiled extension.
///
/// # Errors
///
/// Returns an error when the module entry source cannot be read, parsed, or the
/// stub cannot be written.
pub fn write_module_stub_for_extension(
    config: &ProjectConfig,
    module: &MojoModule,
    extension_output: &Path,
) -> Result<PathBuf> {
    let stub = module_stub_plan_for_extension(config, module, extension_output)?;
    write_file(&stub.path, stub.text.as_bytes())?;
    Ok(stub.path)
}

/// Render a `.pyi` file from Python bindings declared in a Mojo entry file.
///
/// # Errors
///
/// Returns an error when the configured module entry source cannot be read or
/// any exported binding references an unsupported or unresolved declaration.
pub fn render_module_stub(config: &ProjectConfig, module: &MojoModule) -> Result<String> {
    let entry = config.project_dir.join(&module.entry);
    let sources = read_stub_sources(config, &entry)?;
    render_stub_from_sources(&sources)
        .map_err(|message| invalid_stub_source(&entry, module.name.as_str(), message))
}

fn render_stub_from_sources(sources: &[StubSource]) -> std::result::Result<String, String> {
    let parsed = parse_sources(sources)?;
    let bindings = resolve_bindings(&parsed)?;
    Ok(render_stub_text(&bindings))
}

fn module_stub_path_in_dir(module: &MojoModule, output_dir: &Path) -> PathBuf {
    output_dir.join(format!("{}.pyi", module.name.leaf()))
}

fn invalid_stub_source(path: &Path, module_name: &str, message: String) -> MohausError {
    MohausError::InvalidProject {
        message: format!(
            "could not generate Python stub for {module_name} from {}: {message}",
            path.display()
        ),
    }
}

fn read_stub_sources(config: &ProjectConfig, entry: &Path) -> Result<Vec<StubSource>> {
    let mut source_paths = Vec::new();
    let mojo_source_root = config.mojo_source_root();
    collect_mojo_source_files(&mojo_source_root, &mut source_paths)?;
    for include in &config.mojo_include_paths {
        let include_root = if include.is_absolute() {
            include.clone()
        } else {
            config.project_dir.join(include)
        };
        collect_mojo_source_files(&include_root, &mut source_paths)?;
    }
    source_paths.sort();
    source_paths.dedup();

    if !source_paths.iter().any(|path| path == entry) {
        source_paths.push(entry.to_path_buf());
    }

    let mut sources = Vec::new();
    for path in source_paths {
        let text = fs::read_to_string(&path).map_err(|source| MohausError::ReadFile {
            path: path.clone(),
            source,
        })?;
        sources.push(StubSource {
            module_name: module_name_for_source(&mojo_source_root, &path),
            text,
            is_entry: path == entry,
        });
    }
    Ok(sources)
}

fn collect_mojo_source_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_file() => {
            if is_mojo_source(root) {
                out.push(root.to_path_buf());
            }
            Ok(())
        }
        Ok(metadata) if metadata.is_dir() => {
            collect_mojo_source_dir(root, out)?;
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(MohausError::ReadFile {
            path: root.to_path_buf(),
            source,
        }),
    }
}

fn collect_mojo_source_dir(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(root).map_err(|source| MohausError::ReadFile {
        path: root.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| MohausError::ReadFile {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| MohausError::ReadFile {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            collect_mojo_source_dir(&path, out)?;
        } else if file_type.is_file() && is_mojo_source(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_mojo_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "mojo" | "🔥"))
}

fn module_name_for_source(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = relative
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let last = parts.last_mut()?;
    if let Some(stem) = Path::new(last.as_str()).file_stem() {
        *last = stem.to_string_lossy().to_string();
    }
    let module = parts
        .into_iter()
        .filter(|part| part != "__init__")
        .collect::<Vec<_>>()
        .join(".");
    (!module.is_empty()).then_some(module)
}

fn parse_sources(sources: &[StubSource]) -> std::result::Result<ParsedSource, String> {
    let mut defs = BTreeMap::new();
    let mut field_types = BTreeMap::new();
    let mut imports = BTreeMap::new();
    let mut calls = Vec::new();
    let mut saw_entry = false;

    for source in sources {
        let parsed =
            parse_source_unit(&source.text, source.module_name.as_deref(), source.is_entry)?;
        defs.extend(parsed.defs);
        merge_field_types(&mut field_types, parsed.field_types);
        if source.is_entry {
            saw_entry = true;
            imports.extend(parsed.imports);
            calls.extend(parsed.calls);
        }
    }

    if !saw_entry {
        return Err("no Mojo module entry source was available for stub generation".to_string());
    }

    Ok(ParsedSource {
        defs,
        field_types,
        imports,
        calls,
    })
}

fn merge_field_types(target: &mut BTreeMap<String, String>, source: BTreeMap<String, String>) {
    for (name, ty) in source {
        insert_unique_field_type(target, name, ty);
    }
}

fn parse_source_unit(
    source: &str,
    module_name: Option<&str>,
    collect_exports: bool,
) -> std::result::Result<ParsedSource, String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut defs = BTreeMap::new();
    let mut field_types = BTreeMap::new();
    let imports = if collect_exports {
        parse_imports(source)?
    } else {
        BTreeMap::new()
    };
    let mut binding_source = String::new();
    let mut structs: Vec<(usize, String)> = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let line = strip_comment(lines[index]);
        let trimmed = line.trim();
        let indent = indentation(lines[index]);
        if !trimmed.is_empty() {
            while structs
                .last()
                .is_some_and(|(struct_indent, _)| indent <= *struct_indent)
            {
                structs.pop();
            }
        }

        if let Some(name) = parse_struct_name(trimmed) {
            structs.push((indent, name));
        }

        if let Some((struct_indent, struct_name)) = structs.last()
            && indent > *struct_indent
            && indent - *struct_indent <= 4
            && let Some((field, ty)) = parse_field_decl(trimmed)?
        {
            insert_field_type(&mut field_types, struct_name, &field, &ty);
        }

        if starts_def(trimmed) {
            let (header, consumed) = collect_def_header(&lines, index)?;
            let body = collect_def_body(&lines, consumed, indent);
            let mut mojo_def = parse_def_header(&header)?;
            mojo_def.def.body = body;
            let struct_name = structs.last().map(|(_, name)| name.clone());
            mojo_def.def.owner = struct_name.clone();
            insert_def(&mut defs, module_name, None, &mojo_def, collect_exports);
            if let Some(struct_name) = struct_name {
                insert_def(
                    &mut defs,
                    module_name,
                    Some(&struct_name),
                    &mojo_def,
                    collect_exports,
                );
            }
            index = consumed;
            continue;
        }

        if collect_exports {
            binding_source.push_str(line);
            binding_source.push('\n');
        }
        index += 1;
    }

    let calls = if collect_exports {
        parse_binding_calls(&binding_source)?
    } else {
        Vec::new()
    };
    Ok(ParsedSource {
        defs,
        field_types,
        imports,
        calls,
    })
}

fn insert_def(
    defs: &mut BTreeMap<String, MojoDef>,
    module_name: Option<&str>,
    struct_name: Option<&str>,
    mojo_def: &NamedDef,
    is_entry: bool,
) {
    let local_name = struct_name.map_or_else(
        || mojo_def.name.clone(),
        |struct_name| format!("{struct_name}.{}", mojo_def.name),
    );
    if is_entry || struct_name.is_some() || module_name.is_none() {
        defs.insert(local_name.clone(), mojo_def.def.clone());
    }
    if let Some(module_name) = module_name {
        defs.insert(format!("{module_name}.{local_name}"), mojo_def.def.clone());
    }
}

fn parse_field_decl(trimmed: &str) -> std::result::Result<Option<(String, String)>, String> {
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
    Ok(Some((clean_identifier(name)?, normalize_mojo_type(ty))))
}

fn insert_field_type(
    field_types: &mut BTreeMap<String, String>,
    struct_name: &str,
    field_name: &str,
    ty: &str,
) {
    insert_unique_field_type(field_types, field_name.to_string(), ty.to_string());
    insert_unique_field_type(
        field_types,
        format!("{struct_name}.{field_name}"),
        ty.to_string(),
    );
}

fn insert_unique_field_type(field_types: &mut BTreeMap<String, String>, name: String, ty: String) {
    match field_types.get_mut(&name) {
        Some(existing) if existing != &ty => existing.clear(),
        Some(_) => {}
        None => {
            field_types.insert(name, ty);
        }
    }
}

fn parse_imports(source: &str) -> std::result::Result<BTreeMap<String, String>, String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut imports = BTreeMap::new();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = strip_comment(lines[index]).trim();
        let Some(rest) = trimmed.strip_prefix("from ") else {
            index += 1;
            continue;
        };
        let Some((module, raw_names)) = rest.split_once(" import ") else {
            index += 1;
            continue;
        };

        let mut names = raw_names.trim().to_string();
        if names.starts_with('(') && matching_delimiter(&names, 0, '(', ')').is_none() {
            let start = index + 1;
            index = start;
            while index < lines.len() {
                names.push(' ');
                names.push_str(strip_comment(lines[index]).trim());
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

        insert_imports(&mut imports, module.trim(), &names);
        index += 1;
    }
    Ok(imports)
}

fn insert_imports(imports: &mut BTreeMap<String, String>, module: &str, raw_names: &str) {
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
    for item in split_top_level_commas(names) {
        let item = item.trim();
        if item.is_empty() || item == "*" {
            continue;
        }
        let parts = item.split_whitespace().collect::<Vec<_>>();
        let Some(original) = parts.first() else {
            continue;
        };
        let alias = if parts.len() >= 3 && parts[1] == "as" {
            parts[2]
        } else {
            original
        };
        if valid_python_identifier(alias) && valid_python_identifier(original) {
            imports.insert(alias.to_string(), format!("{module}.{original}"));
        }
    }
}

fn strip_comment(line: &str) -> &str {
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

fn indentation(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct NamedDef {
    name: String,
    def: MojoDef,
}

fn collect_def_header(
    lines: &[&str],
    start: usize,
) -> std::result::Result<(String, usize), String> {
    let mut header = String::new();
    let mut index = start;
    while index < lines.len() {
        if !header.is_empty() {
            header.push(' ');
        }
        header.push_str(strip_comment(lines[index]).trim());
        if top_level_colon_index(&header).is_some() {
            return Ok((header, index + 1));
        }
        index += 1;
    }
    Err(format!(
        "unterminated def header starting at line {}",
        start + 1
    ))
}

fn collect_def_body(lines: &[&str], start: usize, def_indent: usize) -> String {
    let mut body = String::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = strip_comment(line).trim();
        if !trimmed.is_empty() && indentation(line) <= def_indent {
            break;
        }
        body.push_str(line);
        body.push('\n');
        index += 1;
    }
    body
}

fn parse_def_header(header: &str) -> std::result::Result<NamedDef, String> {
    let colon =
        top_level_colon_index(header).ok_or_else(|| "def header has no colon".to_string())?;
    let header = header[..colon].trim();
    let rest = header
        .strip_prefix("def ")
        .ok_or_else(|| format!("not a def header: {header}"))?
        .trim_start();
    let open = find_top_level_char(rest, '(')
        .ok_or_else(|| format!("def header has no parameter list: {header}"))?;
    let raw_name = rest[..open].trim();
    let name = clean_identifier(strip_generic_params(raw_name))?;
    let close = matching_delimiter(rest, open, '(', ')')
        .ok_or_else(|| format!("def `{name}` has an unterminated parameter list"))?;
    let params_text = &rest[open + 1..close];
    let params = parse_params(params_text)?;
    let suffix = rest[close + 1..].trim();
    let return_type = suffix
        .find("->")
        .map(|arrow| suffix[arrow + 2..].trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(NamedDef {
        name,
        def: MojoDef {
            params,
            return_type,
            body: String::new(),
            owner: None,
        },
    })
}

fn strip_generic_params(raw_name: &str) -> &str {
    if let Some(bracket) = find_top_level_char(raw_name, '[') {
        raw_name[..bracket].trim_end()
    } else {
        raw_name
    }
}

fn parse_params(params: &str) -> std::result::Result<Vec<MojoParam>, String> {
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
        parsed.push(MojoParam {
            name: clean_identifier(name)?,
            ty: ty_part.trim().to_string(),
        });
    }
    Ok(parsed)
}

fn strip_default(raw: &str) -> &str {
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

fn top_level_colon_index(text: &str) -> Option<usize> {
    find_top_level_char(text, ':')
}

fn find_top_level_char(text: &str, needle: char) -> Option<usize> {
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

fn matching_delimiter(text: &str, open: usize, start: char, end: char) -> Option<usize> {
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

fn split_top_level_commas(text: &str) -> Vec<&str> {
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

fn clean_identifier(raw: &str) -> std::result::Result<String, String> {
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

fn parse_binding_calls(line: &str) -> std::result::Result<Vec<BindingCall>, String> {
    let mut calls = Vec::new();
    for (name, kind) in BINDING_CALLS {
        let mut offset = 0;
        while let Some(found) = line[offset..].find(name) {
            let start = offset + found;
            if !is_call_boundary(line, start, name.len()) {
                offset = start + name.len();
                continue;
            }
            if let Some((call, end)) = parse_binding_call(line, start, *kind, name)? {
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

fn is_call_boundary(line: &str, start: usize, len: usize) -> bool {
    let before = line[..start].chars().next_back();
    let after = line[start + len..].chars().next();
    let before_ok = before.is_none_or(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()));
    let after_ok = after.is_none_or(|ch| matches!(ch, '[' | '('));
    before_ok && after_ok
}

fn parse_binding_call(
    line: &str,
    start: usize,
    kind: BindingKind,
    name: &str,
) -> std::result::Result<Option<(BindingCall, usize)>, String> {
    let mut cursor = start + name.len();
    let generic = if line[cursor..].starts_with('[') {
        let close = matching_delimiter(line, cursor, '[', ']')
            .ok_or_else(|| format!("binding `{name}` has unterminated generic args"))?;
        let value = split_top_level_commas(&line[cursor + 1..close])
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
    if !line[cursor..].trim_start().starts_with('(') {
        return Ok(None);
    }
    cursor += line[cursor..].find('(').unwrap_or_default();
    let close = matching_delimiter(line, cursor, '(', ')')
        .ok_or_else(|| format!("binding `{name}` has unterminated call args"))?;
    let visible_name = parse_first_string_arg(&line[cursor + 1..close]);
    Ok(Some((
        BindingCall {
            kind,
            generic,
            visible_name,
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

fn parse_string_literal(text: &str) -> Option<String> {
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

fn resolve_bindings(parsed: &ParsedSource) -> std::result::Result<StubBindings, String> {
    let mut bindings = StubBindings::default();
    let mut current_class: Option<String> = None;

    for call in &parsed.calls {
        match call.kind {
            BindingKind::AddType => {
                let class_name = required_visible_name(call, "add_type")?;
                require_stub_identifier(&class_name)?;
                bindings.classes.entry(class_name.clone()).or_default();
                current_class = Some(class_name);
            }
            BindingKind::Function => {
                let visible_name = required_visible_name(call, "def_function")?;
                require_stub_identifier(&visible_name)?;
                let target = required_generic(call, "def_function")?;
                let mojo_def = resolve_def(parsed, &target)?;
                let function = stub_function_from_def(parsed, mojo_def, 0, false)?;
                bindings.functions.insert(visible_name, function);
            }
            BindingKind::PyFunction | BindingKind::PyCFunction => {
                let name = required_visible_name(call, "def_py_function")?;
                require_stub_identifier(&name)?;
                bindings
                    .functions
                    .insert(name, StubFunction::varargs("object"));
            }
            BindingKind::Method => {
                let (class_name, method_name) = class_and_visible(call, current_class.as_deref())?;
                let target = required_generic(call, "def_method")?;
                let mojo_def = resolve_def(parsed, &target)?;
                let function = stub_function_from_def(parsed, mojo_def, 1, false)?;
                bindings
                    .classes
                    .entry(class_name)
                    .or_default()
                    .methods
                    .insert(method_name, function);
            }
            BindingKind::StaticMethod => {
                let (class_name, method_name) = class_and_visible(call, current_class.as_deref())?;
                let target = required_generic(call, "def_staticmethod")?;
                let mojo_def = resolve_def(parsed, &target)?;
                let function = stub_function_from_def(parsed, mojo_def, 0, false)?;
                bindings
                    .classes
                    .entry(class_name)
                    .or_default()
                    .static_methods
                    .insert(method_name, function);
            }
            BindingKind::PyMethod | BindingKind::PyCMethod => {
                let (class_name, method_name) = class_and_visible(call, current_class.as_deref())?;
                bindings
                    .classes
                    .entry(class_name)
                    .or_default()
                    .methods
                    .insert(method_name, StubFunction::varargs("object"));
            }
            BindingKind::PyInit => {
                let class_name =
                    class_for_type_call(call, current_class.as_deref(), "def_py_init")?;
                bindings.classes.entry(class_name).or_default().init =
                    Some(StubFunction::varargs("None"));
            }
            BindingKind::DefaultInit => {
                let class_name =
                    class_for_type_call(call, current_class.as_deref(), "def_init_defaultable")?;
                bindings.classes.entry(class_name).or_default().init = Some(StubFunction {
                    params: Vec::new(),
                    returns: "None".to_string(),
                    varargs: false,
                });
            }
        }
    }

    Ok(bindings)
}

fn required_visible_name(call: &BindingCall, binding: &str) -> std::result::Result<String, String> {
    call.visible_name
        .clone()
        .ok_or_else(|| format!("`{binding}` is missing a Python-visible string name"))
}

fn required_generic(call: &BindingCall, binding: &str) -> std::result::Result<String, String> {
    call.generic
        .clone()
        .ok_or_else(|| format!("`{binding}` is missing a Mojo target in brackets"))
}

fn resolve_def<'a>(
    parsed: &'a ParsedSource,
    target: &str,
) -> std::result::Result<&'a MojoDef, String> {
    let target = target.trim();
    if let Some(direct) = parsed.defs.get(target) {
        return Ok(direct);
    }
    if !target.contains('.')
        && let Some(imported_target) = parsed.imports.get(target)
        && let Some(imported) = parsed.defs.get(imported_target)
    {
        return Ok(imported);
    }
    let leaf = target
        .rsplit('.')
        .next()
        .and_then(|name| parsed.defs.get(name));
    leaf.ok_or_else(|| {
        format!("exported binding target `{target}` does not resolve to a local Mojo `def`")
    })
}

fn class_and_visible(
    call: &BindingCall,
    current_class: Option<&str>,
) -> std::result::Result<(String, String), String> {
    let method_name = required_visible_name(call, "method binding")?;
    require_stub_identifier(&method_name)?;
    let class_name = class_for_type_call(call, current_class, "method binding")?;
    Ok((class_name, method_name))
}

fn class_for_type_call(
    call: &BindingCall,
    current_class: Option<&str>,
    binding: &str,
) -> std::result::Result<String, String> {
    if let Some(generic) = &call.generic
        && let Some(class_name) = generic.rsplit_once('.').map(|(class, _)| class)
    {
        let class_name = class_name.rsplit('.').next().unwrap_or(class_name);
        require_stub_identifier(class_name)?;
        return Ok(class_name.to_string());
    }
    if let Some(class_name) = current_class {
        return Ok(class_name.to_string());
    }
    Err(format!(
        "`{binding}` is not attached to a discoverable `add_type`"
    ))
}

fn stub_function_from_def(
    parsed: &ParsedSource,
    mojo_def: &MojoDef,
    drop_params: usize,
    varargs: bool,
) -> std::result::Result<StubFunction, String> {
    if varargs {
        return Ok(StubFunction::varargs("object"));
    }
    let mut params = Vec::new();
    let binding_params = mojo_def.params.iter().skip(drop_params).collect::<Vec<_>>();
    let binding_param_count = binding_params.len();
    for (index, param) in binding_params.into_iter().enumerate() {
        let normalized = normalize_mojo_type(&param.ty);
        if is_owned_kwargs_dict(&normalized) {
            if index + 1 != binding_param_count {
                return Err(format!(
                    "keyword dict parameter `{}` must be the trailing Python binding argument",
                    param.name
                ));
            }
            params.push(StubParam {
                name: "kwargs".to_string(),
                annotation: "object".to_string(),
                keyword_rest: true,
            });
            continue;
        }
        require_stub_identifier(&param.name)?;
        let annotation = python_type_for_mojo(&normalized)
            .ok_or_else(|| format!("unsupported Python binding parameter type `{}`", param.ty))?;
        params.push(StubParam {
            name: param.name.clone(),
            annotation,
            keyword_rest: false,
        });
    }
    let returns = match mojo_def.return_type.as_deref().map(normalize_mojo_type) {
        Some(ty) => python_type_for_mojo_return(parsed, mojo_def, &ty)
            .ok_or_else(|| format!("unsupported Python binding return type `{ty}`"))?,
        None => "None".to_string(),
    };
    Ok(StubFunction {
        params,
        returns,
        varargs: false,
    })
}

fn normalize_mojo_type(ty: &str) -> String {
    ty.trim()
        .trim_start_matches("mut ")
        .trim_start_matches("owned ")
        .trim()
        .to_string()
}

fn is_owned_kwargs_dict(ty: &str) -> bool {
    ty.starts_with("OwnedKwargsDict") && ty.contains("PythonObject")
}

fn python_type_for_mojo(ty: &str) -> Option<String> {
    match ty {
        "PythonObject" => Some("object".to_string()),
        "Bool" => Some("bool".to_string()),
        "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt" | "UInt8" | "UInt16" | "UInt32"
        | "UInt64" => Some("int".to_string()),
        "Float16" | "Float32" | "Float64" => Some("float".to_string()),
        "String" | "StringSlice" => Some("str".to_string()),
        _ => None,
    }
}

fn python_type_for_mojo_return(
    parsed: &ParsedSource,
    mojo_def: &MojoDef,
    ty: &str,
) -> Option<String> {
    if ty == "None" {
        Some("None".to_string())
    } else if ty == "PythonObject" {
        Some(infer_python_object_return(parsed, mojo_def).unwrap_or_else(|| "object".to_string()))
    } else {
        python_type_for_mojo(ty)
    }
}

fn infer_python_object_return(parsed: &ParsedSource, mojo_def: &MojoDef) -> Option<String> {
    infer_python_object_return_inner(parsed, mojo_def, &mut Vec::new())
}

fn infer_python_object_return_inner(
    parsed: &ParsedSource,
    mojo_def: &MojoDef,
    stack: &mut Vec<String>,
) -> Option<String> {
    let mut types = Vec::new();
    for expression in return_expressions(&mojo_def.body) {
        let inferred = infer_python_object_return_expression(parsed, mojo_def, &expression, stack)?;
        push_unique_type(&mut types, inferred);
    }
    (!types.is_empty()).then(|| types.join(" | "))
}

fn return_expressions(body: &str) -> Vec<String> {
    let mut expressions = Vec::new();
    for line in body.lines() {
        let trimmed = strip_comment(line).trim();
        if let Some(expression) = trimmed.strip_prefix("return ") {
            expressions.push(expression.trim().to_string());
        }
    }
    expressions
}

fn push_unique_type(types: &mut Vec<String>, ty: String) {
    if !types.iter().any(|existing| existing == &ty) {
        types.push(ty);
    }
}

fn infer_python_object_return_expression(
    parsed: &ParsedSource,
    mojo_def: &MojoDef,
    expression: &str,
    stack: &mut Vec<String>,
) -> Option<String> {
    let expression = expression.trim();
    if expression == "PythonObject.none()" {
        return Some("None".to_string());
    }
    if let Some(inner) = call_inner(expression, "PythonObject") {
        let args = split_top_level_commas(inner);
        for arg in &args {
            if let Some(alloc_expr) = arg.trim().strip_prefix("alloc=") {
                return infer_alloc_python_type(parsed, mojo_def, alloc_expr);
            }
        }
        let first = args.first()?.trim();
        return if first == "None" {
            Some("None".to_string())
        } else {
            infer_python_value_type(parsed, mojo_def, first)
        };
    }

    let call = call_name(expression)?;
    if stack.iter().any(|seen| seen == &call) {
        return None;
    }
    let target = resolve_inferred_def(parsed, &call)?;
    let return_type = target.return_type.as_deref().map(normalize_mojo_type)?;
    if return_type == "PythonObject" {
        stack.push(call);
        let inferred = infer_python_object_return_inner(parsed, target, stack);
        stack.pop();
        inferred
    } else {
        python_type_for_mojo(&return_type)
    }
}

fn call_inner<'a>(expression: &'a str, name: &str) -> Option<&'a str> {
    let rest = expression.strip_prefix(name)?;
    if !rest.starts_with('(') {
        return None;
    }
    let open = name.len();
    let close = matching_delimiter(expression, open, '(', ')')?;
    expression[close + 1..]
        .trim()
        .is_empty()
        .then_some(&expression[open + 1..close])
}

fn infer_alloc_python_type(
    parsed: &ParsedSource,
    mojo_def: &MojoDef,
    expression: &str,
) -> Option<String> {
    let expression = strip_mojo_move(expression);
    if valid_python_identifier(expression)
        && let Some(mojo_type) = infer_local_variable_mojo_type(parsed, &mojo_def.body, expression)
        && let Some(python_type) = python_alloc_type_for_mojo(&mojo_type)
    {
        return Some(python_type);
    }
    infer_mojo_expression_type(parsed, expression)
        .and_then(|ty| python_alloc_type_for_mojo(&ty))
        .or_else(|| single_bound_class(parsed))
}

fn strip_mojo_move(expression: &str) -> &str {
    expression
        .trim()
        .strip_suffix('^')
        .unwrap_or(expression.trim())
        .trim()
}

fn infer_local_variable_mojo_type(
    parsed: &ParsedSource,
    body: &str,
    variable: &str,
) -> Option<String> {
    for line in body.lines() {
        let trimmed = strip_comment(line).trim();
        let Some(rest) = trimmed
            .strip_prefix("var ")
            .or_else(|| trimmed.strip_prefix("let "))
        else {
            continue;
        };
        let Some((left, right)) = rest.split_once('=') else {
            continue;
        };
        let left = left.trim();
        let name = left.split_once(':').map_or(left, |(name, _)| name).trim();
        if name != variable {
            continue;
        }
        if let Some((_, explicit_type)) = left.split_once(':') {
            return Some(normalize_mojo_type(explicit_type));
        }
        if let Some(inferred) = infer_mojo_expression_type(parsed, right.trim()) {
            return Some(inferred);
        }
    }
    None
}

fn infer_python_value_type(
    parsed: &ParsedSource,
    mojo_def: &MojoDef,
    expression: &str,
) -> Option<String> {
    let expression = expression.trim();
    if expression == "None" {
        return Some("None".to_string());
    }
    if matches!(expression, "True" | "False") {
        return Some("bool".to_string());
    }
    if parse_string_literal(expression).is_some() {
        return Some("str".to_string());
    }
    if is_int_literal(expression) {
        return Some("int".to_string());
    }
    if is_float_literal(expression) {
        return Some("float".to_string());
    }
    if expression.ends_with(".__int__()") || expression.ends_with(".__index__()") {
        return Some("int".to_string());
    }
    if contains_comparison(expression) {
        return Some("bool".to_string());
    }
    if let Some(call) = call_name(expression) {
        if let Some(ty) = python_type_for_builtin_mojo_constructor(&call) {
            return Some(ty);
        }
        if let Some(ty) =
            infer_mojo_call_return_type(parsed, &call).and_then(|ty| python_type_for_mojo(&ty))
        {
            return Some(ty);
        }
    }
    if valid_python_identifier(expression)
        && let Some(ty) = infer_local_variable_mojo_type(parsed, &mojo_def.body, expression)
    {
        return python_type_for_mojo(&ty);
    }
    infer_field_access_python_type(parsed, mojo_def, expression)
}

fn is_int_literal(expression: &str) -> bool {
    let value = expression.strip_prefix('-').unwrap_or(expression);
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

fn is_float_literal(expression: &str) -> bool {
    let value = expression.strip_prefix('-').unwrap_or(expression);
    value.contains('.')
        && value.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        && value.chars().any(|ch| ch.is_ascii_digit())
}

fn contains_comparison(expression: &str) -> bool {
    [" == ", " != ", " <= ", " >= ", " < ", " > "]
        .iter()
        .any(|operator| expression.contains(operator))
}

fn python_type_for_builtin_mojo_constructor(name: &str) -> Option<String> {
    match name {
        "Bool" => Some("bool".to_string()),
        "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt" | "UInt8" | "UInt16" | "UInt32"
        | "UInt64" | "len" => Some("int".to_string()),
        "Float16" | "Float32" | "Float64" => Some("float".to_string()),
        "String" | "StringSlice" => Some("str".to_string()),
        _ => None,
    }
}

fn infer_mojo_expression_type(parsed: &ParsedSource, expression: &str) -> Option<String> {
    let call = call_name(strip_mojo_move(expression))?;
    infer_mojo_call_return_type(parsed, &call)
}

fn infer_mojo_call_return_type(parsed: &ParsedSource, call: &str) -> Option<String> {
    resolve_inferred_def(parsed, call.trim())
        .and_then(|mojo_def| mojo_def.return_type.as_ref())
        .map(|ty| normalize_mojo_type(ty))
}

fn resolve_inferred_def<'a>(parsed: &'a ParsedSource, call: &str) -> Option<&'a MojoDef> {
    if let Some(direct) = parsed.defs.get(call) {
        return Some(direct);
    }
    let leaf = call.rsplit('.').next().unwrap_or(call).trim();
    let mut match_def: Option<&MojoDef> = None;
    for (name, mojo_def) in &parsed.defs {
        if name.rsplit('.').next() != Some(leaf) {
            continue;
        }
        match match_def {
            Some(existing) if existing != mojo_def => return None,
            Some(_) => {}
            None => match_def = Some(mojo_def),
        }
    }
    match_def
}

fn call_name(expression: &str) -> Option<String> {
    let open = find_top_level_char(expression, '(')?;
    let head = strip_generic_params(expression[..open].trim()).trim();
    let leaf = head.rsplit('.').next().unwrap_or(head).trim();
    valid_python_identifier(leaf).then(|| leaf.to_string())
}

fn python_alloc_type_for_mojo(ty: &str) -> Option<String> {
    let normalized = normalize_mojo_type(ty);
    if normalized.contains('[') || python_type_for_mojo(&normalized).is_some() {
        return None;
    }
    let leaf = normalized.rsplit('.').next().unwrap_or(&normalized);
    valid_python_identifier(leaf).then(|| leaf.to_string())
}

fn single_bound_class(parsed: &ParsedSource) -> Option<String> {
    let mut classes = Vec::new();
    for call in &parsed.calls {
        if call.kind == BindingKind::AddType
            && let Some(name) = &call.visible_name
            && !classes.iter().any(|existing| existing == name)
        {
            classes.push(name.clone());
        }
    }
    (classes.len() == 1).then(|| classes.remove(0))
}

fn infer_field_access_python_type(
    parsed: &ParsedSource,
    mojo_def: &MojoDef,
    expression: &str,
) -> Option<String> {
    if let Some(owner) = &mojo_def.owner
        && let Some(python_type) = infer_owner_field_access_python_type(parsed, owner, expression)
    {
        return Some(python_type);
    }
    for (field, ty) in &parsed.field_types {
        if field.contains('.') {
            continue;
        }
        if let Some(python_type) = infer_field_python_type(expression, field, ty) {
            return Some(python_type);
        }
    }
    None
}

fn infer_owner_field_access_python_type(
    parsed: &ParsedSource,
    owner: &str,
    expression: &str,
) -> Option<String> {
    let prefix = format!("{owner}.");
    for (field, ty) in &parsed.field_types {
        let Some(field) = field.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(python_type) = infer_field_python_type(expression, field, ty) {
            return Some(python_type);
        }
    }
    None
}

fn infer_field_python_type(expression: &str, field: &str, ty: &str) -> Option<String> {
    if ty.is_empty() {
        return None;
    }
    let dotted = format!(".{field}");
    let bracketed = format!("[].{field}");
    if !expression.contains(&dotted) && !expression.contains(&bracketed) {
        return None;
    }
    let indexed =
        expression.contains(&format!(".{field}[")) || expression.contains(&format!("[].{field}["));
    let mojo_type = if indexed {
        list_item_type(ty).unwrap_or(ty)
    } else {
        ty
    };
    python_type_for_mojo(mojo_type)
}

fn list_item_type(ty: &str) -> Option<&str> {
    ty.strip_prefix("List[")?.strip_suffix(']').map(str::trim)
}

fn render_stub_text(bindings: &StubBindings) -> String {
    let mut text = String::new();
    let mut wrote_item = false;
    for (name, function) in &bindings.functions {
        text.push_str(&render_function("def", name, function, 0, false));
        wrote_item = true;
    }

    for (name, class) in &bindings.classes {
        if wrote_item {
            text.push('\n');
        }
        text.push_str(&format!("class {name}:\n"));
        let mut wrote_class_item = false;
        if let Some(init) = &class.init {
            text.push_str(&render_function("def", "__init__", init, 2, true));
            wrote_class_item = true;
        }
        for (method, function) in &class.methods {
            text.push_str(&render_function("def", method, function, 2, true));
            wrote_class_item = true;
        }
        for (method, function) in &class.static_methods {
            text.push_str("  @staticmethod\n");
            text.push_str(&render_function("def", method, function, 2, false));
            wrote_class_item = true;
        }
        if !wrote_class_item {
            text.push_str("  ...\n");
        }
        wrote_item = true;
    }

    if !wrote_item {
        text.push_str("...\n");
    }
    text
}

fn render_function(
    prefix: &str,
    name: &str,
    function: &StubFunction,
    indent: usize,
    include_self: bool,
) -> String {
    let mut params = Vec::new();
    if include_self {
        params.push("self".to_string());
    }
    if function.varargs {
        params.push("*args: object".to_string());
        params.push("**kwargs: object".to_string());
    } else {
        params.extend(function.params.iter().map(|param| {
            if param.keyword_rest {
                format!("**{}: {}", param.name, param.annotation)
            } else {
                format!("{}: {}", param.name, param.annotation)
            }
        }));
    }
    format!(
        "{}{prefix} {name}({}) -> {}: ...\n",
        " ".repeat(indent),
        params.join(", "),
        function.returns
    )
}

fn require_stub_identifier(value: &str) -> std::result::Result<(), String> {
    if valid_python_identifier(value) {
        Ok(())
    } else {
        Err(format!(
            "`{value}` is not a supported Python stub identifier"
        ))
    }
}

fn valid_python_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && !is_python_keyword(value)
}

fn is_python_keyword(value: &str) -> bool {
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
    use std::fs;

    use tempfile::TempDir;

    use crate::config::ProjectConfig;
    use crate::stub::{
        StubSource, module_stub_plan_for_extension, render_module_stub, render_stub_from_sources,
    };

    fn render_stub_from_source(source: &str) -> std::result::Result<String, String> {
        render_stub_from_sources(&[StubSource {
            module_name: None,
            text: source.to_string(),
            is_entry: true,
        }])
    }

    #[test]
    fn renders_function_and_class_bindings() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    _ = (
        module.def_function[passthrough]("passthrough")
        .add_type[Greeter]("Greeter")
        .def_py_init[Greeter.py_init]()
        .def_method[Greeter.greet]("greet")
    )
    return module.finalize()

def passthrough(value: PythonObject) raises -> PythonObject:
    return value

@fieldwise_init
struct Greeter(Movable, Writable):
    var suffix: String

    @staticmethod
    def py_init(out self: Greeter, args: PythonObject, kwargs: PythonObject) raises:
        self = Self(String(py=args[0]))

    @staticmethod
    def greet(py_self: PythonObject, value: PythonObject) raises -> PythonObject:
        return value
"#,
        )
        .unwrap();

        assert_eq!(
            text,
            concat!(
                "def passthrough(value: object) -> object: ...\n\n",
                "class Greeter:\n",
                "  def __init__(self, *args: object, **kwargs: object) -> None: ...\n",
                "  def greet(self, value: object) -> object: ...\n",
            )
        );
    }

    #[test]
    fn renders_staticmethod_default_init_and_keyword_dict() {
        let text = render_stub_from_source(
            r#"
from std.collections import OwnedKwargsDict
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

def duration(hours: PythonObject, kwargs: OwnedKwargsDict[PythonObject]) raises -> PythonObject:
    return hours

struct Timer(Defaultable, Movable, Writable):
    @staticmethod
    def is_valid(value: PythonObject) raises -> PythonObject:
        return value

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[duration]("duration")
    _ = (
        module.add_type[Timer]("Timer")
        .def_init_defaultable[Timer]()
        .def_staticmethod[Timer.is_valid]("is_valid")
    )
    return module.finalize()
"#,
        )
        .unwrap();

        assert_eq!(
            text,
            concat!(
                "def duration(hours: object, **kwargs: object) -> object: ...\n\n",
                "class Timer:\n",
                "  def __init__(self) -> None: ...\n",
                "  @staticmethod\n",
                "  def is_valid(value: object) -> object: ...\n",
            )
        );
    }

    #[test]
    fn broadens_low_level_py_and_c_bindings_to_object_varargs() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

def count_args(py_self: PythonObject, args_tuple: PythonObject) raises -> PythonObject:
    return args_tuple

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_py_function[count_args]("count_args")
    module.def_py_c_function(raw_count, "raw_count")
    _ = (
        module.add_type[Counter]("Counter")
        .def_py_method[Counter.lookup]("lookup")
        .def_py_c_method(raw_lookup, "raw_lookup")
    )
    return module.finalize()

struct Counter(Movable, Writable):
    @staticmethod
    def lookup(py_self: PythonObject, args_tuple: PythonObject) raises -> PythonObject:
        return args_tuple
"#,
        )
        .unwrap();

        assert_eq!(
            text,
            concat!(
                "def count_args(*args: object, **kwargs: object) -> object: ...\n",
                "def raw_count(*args: object, **kwargs: object) -> object: ...\n\n",
                "class Counter:\n",
                "  def lookup(self, *args: object, **kwargs: object) -> object: ...\n",
                "  def raw_lookup(self, *args: object, **kwargs: object) -> object: ...\n",
            )
        );
    }

    #[test]
    fn parses_multiline_chained_builder_calls() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

def passthrough(value: PythonObject) raises -> PythonObject:
    return value

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    _ = (
        module
            .def_function[
                passthrough,
            ](
                "passthrough",
            )
    )
    return module.finalize()
"#,
        )
        .unwrap();

        assert_eq!(text, "def passthrough(value: object) -> object: ...\n");
    }

    #[test]
    fn parses_generic_def_headers_without_treating_brackets_as_names() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

def passthrough[T: CollectionElement](value: PythonObject) raises -> PythonObject:
    return value

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[passthrough]("passthrough")
    return module.finalize()
"#,
        )
        .unwrap();

        assert_eq!(text, "def passthrough(value: object) -> object: ...\n");
    }

    #[test]
    fn infers_pythonobject_wrapper_return_types() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder
from std.collections import List

struct Layout:
    var shape: Shape[Self.rank]

struct Array(Movable, Writable):
    var dtype_code: Int
    var shape: List[Int]

    @staticmethod
    def dtype_code_py(py_self: PythonObject) raises -> PythonObject:
        var self_ptr = py_self.downcast_value_ptr[Self]()
        return PythonObject(self_ptr[].dtype_code)

    @staticmethod
    def shape_at_py(py_self: PythonObject, index_obj: PythonObject) raises -> PythonObject:
        var self_ptr = py_self.downcast_value_ptr[Self]()
        var index = Int(py=index_obj)
        return PythonObject(self_ptr[].shape[index])

    @staticmethod
    def get_scalar_py(py_self: PythonObject, index_obj: PythonObject) raises -> PythonObject:
        if True:
            return PythonObject(get_bool())
        if False:
            return PythonObject(get_i64())
        return PythonObject(get_f64())

    @staticmethod
    def used_fast_py(py_self: PythonObject) raises -> PythonObject:
        var self_ptr = py_self.downcast_value_ptr[Self]()
        return PythonObject(self_ptr[].dtype_code == 1)

    @staticmethod
    def is_c_contiguous_py(py_self: PythonObject) raises -> PythonObject:
        return PythonObject(is_c_contiguous())

def is_c_contiguous() -> Bool:
    return True

def make_array() raises -> Array:
    pass

def get_bool() -> Bool:
    return True

def get_i64() -> Int64:
    return 1

def get_f64() -> Float64:
    return 1.0

def empty_ops() raises -> PythonObject:
    var result = make_array()
    return PythonObject(alloc=result^)

def none_ops() raises -> PythonObject:
    return PythonObject(None)

def binary_op_method_ops(py_self: PythonObject, other_obj: PythonObject, op: Int) raises -> PythonObject:
    var result = make_array()
    return PythonObject(alloc=result^)

def array_add_method_ops(py_self: PythonObject, other_obj: PythonObject) raises -> PythonObject:
    return binary_op_method_ops(py_self, other_obj, 0)

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    _ = (
        module.add_type[Array]("Array")
        .def_method[array_add_method_ops]("add")
        .def_method[Array.dtype_code_py]("dtype_code")
        .def_method[Array.shape_at_py]("shape_at")
        .def_method[Array.get_scalar_py]("get_scalar")
        .def_method[Array.used_fast_py]("used_fast")
        .def_method[Array.is_c_contiguous_py]("is_c_contiguous")
    )
    module.def_function[empty_ops]("empty")
    module.def_function[none_ops]("none")
    return module.finalize()
"#,
        )
        .unwrap();

        assert_eq!(
            text,
            concat!(
                "def empty() -> Array: ...\n",
                "def none() -> None: ...\n\n",
                "class Array:\n",
                "  def add(self, other_obj: object) -> Array: ...\n",
                "  def dtype_code(self) -> int: ...\n",
                "  def get_scalar(self, index_obj: object) -> bool | int | float: ...\n",
                "  def is_c_contiguous(self) -> bool: ...\n",
                "  def shape_at(self, index_obj: object) -> int: ...\n",
                "  def used_fast(self) -> bool: ...\n",
            )
        );
    }

    #[test]
    fn unresolved_binding_target_is_an_error() {
        let error = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[missing]("missing")
    return module.finalize()
"#,
        )
        .unwrap_err();

        assert!(error.contains("missing"));
        assert!(error.contains("does not resolve"));
    }

    #[test]
    fn targets_extension_leaf_without_abi_suffix() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("python/demo")).unwrap();
        fs::write(temp.path().join(".mojo-version"), "0.26.2.0").unwrap();
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
"#,
        )
        .unwrap();
        fs::write(temp.path().join("src/lib.mojo"), "").unwrap();

        let config = ProjectConfig::load(temp.path()).unwrap();
        let extension = temp
            .path()
            .join("python/demo/_native.cpython-311-darwin.so");
        let stub = module_stub_plan_for_extension(&config, &config.modules[0], &extension).unwrap();

        assert_eq!(stub.path, temp.path().join("python/demo/_native.pyi"));
        assert_eq!(stub.text, "...\n");
    }

    #[test]
    fn render_module_stub_adds_project_context_to_errors() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("python/demo")).unwrap();
        fs::write(temp.path().join(".mojo-version"), "0.26.2.0").unwrap();
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[missing]("missing")
    return module.finalize()
"#,
        )
        .unwrap();

        let config = ProjectConfig::load(temp.path()).unwrap();
        let error = render_module_stub(&config, &config.modules[0]).unwrap_err();

        assert!(error.to_string().contains("demo._native"));
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn resolves_binding_targets_from_source_root_modules() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("python/demo")).unwrap();
        fs::write(temp.path().join(".mojo-version"), "0.26.2.0").unwrap();
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

from array import Array
from create import (
    imported_fun as aliased_fun,
)

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    _ = module.add_type[Array]("Array").def_method[Array.dtype_code_py]("dtype_code")
    module.def_function[aliased_fun]("aliased_fun")
    return module.finalize()
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/array.mojo"),
            r#"
from std.python import PythonObject

struct Array(Movable, Writable):
    @staticmethod
    def dtype_code_py(py_self: PythonObject) raises -> PythonObject:
        return PythonObject(1)
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/create.mojo"),
            r#"
from std.python import PythonObject

def imported_fun(value: PythonObject) raises -> PythonObject:
    return value
"#,
        )
        .unwrap();

        let config = ProjectConfig::load(temp.path()).unwrap();
        let text = render_module_stub(&config, &config.modules[0]).unwrap();

        assert_eq!(
            text,
            concat!(
                "def aliased_fun(value: object) -> object: ...\n\n",
                "class Array:\n",
                "  def dtype_code(self) -> int: ...\n",
            )
        );
    }
}
