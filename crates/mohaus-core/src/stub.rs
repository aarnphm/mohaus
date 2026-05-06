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
    calls: Vec<BindingCall>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MojoDef {
    params: Vec<MojoParam>,
    return_type: Option<String>,
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
    let source = fs::read_to_string(&entry).map_err(|source| MohausError::ReadFile {
        path: entry.clone(),
        source,
    })?;
    render_stub_from_source(&source)
        .map_err(|message| invalid_stub_source(&entry, module.name.as_str(), message))
}

fn render_stub_from_source(source: &str) -> std::result::Result<String, String> {
    let parsed = parse_source(source)?;
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

fn parse_source(source: &str) -> std::result::Result<ParsedSource, String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut defs = BTreeMap::new();
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

        if starts_def(trimmed) {
            let (header, consumed) = collect_def_header(&lines, index)?;
            let mojo_def = parse_def_header(&header)?;
            let struct_name = structs.last().map(|(_, name)| name.clone());
            defs.insert(mojo_def.name.clone(), mojo_def.def.clone());
            if let Some(struct_name) = struct_name {
                defs.insert(format!("{struct_name}.{}", mojo_def.name), mojo_def.def);
            }
            index = consumed;
            continue;
        }

        binding_source.push_str(line);
        binding_source.push('\n');
        index += 1;
    }

    let calls = parse_binding_calls(&binding_source)?;
    Ok(ParsedSource { defs, calls })
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
    let name = clean_identifier(raw_name)?;
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
        },
    })
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
                let function = stub_function_from_def(mojo_def, 0, false)?;
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
                let function = stub_function_from_def(mojo_def, 1, false)?;
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
                let function = stub_function_from_def(mojo_def, 0, false)?;
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
    let direct = parsed.defs.get(target);
    let leaf = target
        .rsplit('.')
        .next()
        .and_then(|name| parsed.defs.get(name));
    direct.or(leaf).ok_or_else(|| {
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
        Some(ty) => python_type_for_mojo_return(&ty)
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

fn python_type_for_mojo_return(ty: &str) -> Option<String> {
    if ty == "None" {
        Some("None".to_string())
    } else {
        python_type_for_mojo(ty)
    }
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
        module_stub_plan_for_extension, render_module_stub, render_stub_from_source,
    };

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
}
