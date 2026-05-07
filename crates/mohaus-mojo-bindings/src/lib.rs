use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mohaus_mojo_syntax::{
    BindingCall, BindingKind, FunctionDecl, ImportDecl, SourceId, Stmt, clean_identifier,
    find_top_level_char, matching_delimiter, normalize_mojo_type, parse_module,
    parse_string_literal, split_top_level_commas, strip_comment, strip_generic_params,
    valid_python_identifier,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ModuleName(pub String);

impl ModuleName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ModuleName {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ModuleName {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceInput {
    pub module_name: Option<ModuleName>,
    pub text: String,
    pub is_entry: bool,
}

impl SourceInput {
    #[must_use]
    pub fn new(module_name: Option<impl Into<ModuleName>>, text: String, is_entry: bool) -> Self {
        Self {
            module_name: module_name.map(Into::into),
            text,
            is_entry,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceGraph {
    pub modules: Vec<SourceModule>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceModule {
    pub module_name: Option<ModuleName>,
    pub ast: mohaus_mojo_syntax::ModuleAst,
    pub is_entry: bool,
}

/// Load and parse a source graph from a Mojo root, include roots, and entry.
///
/// # Errors
///
/// Returns an error when a source file cannot be read or a module cannot be
/// parsed for binding analysis.
pub fn load_source_graph(
    root: &Path,
    include_roots: &[PathBuf],
    entry: &Path,
) -> Result<SourceGraph, String> {
    let mut source_paths = Vec::new();
    collect_mojo_source_files(root, &mut source_paths)?;
    for include_root in include_roots {
        collect_mojo_source_files(include_root, &mut source_paths)?;
    }
    source_paths.sort();
    source_paths.dedup();

    if !source_paths.iter().any(|path| path == entry) {
        source_paths.push(entry.to_path_buf());
    }

    let mut sources = Vec::new();
    for path in source_paths {
        let text = fs::read_to_string(&path)
            .map_err(|source| format!("could not read {}: {source}", path.display()))?;
        sources.push(SourceInput {
            module_name: module_name_for_source(root, &path).map(ModuleName),
            text,
            is_entry: path == entry,
        });
    }
    SourceGraph::parse(&sources)
}

impl SourceGraph {
    /// Parse already-loaded source text into a binding-analysis graph.
    ///
    /// # Errors
    ///
    /// Returns an error when any source fails the clean-room Mojo subset parser.
    pub fn parse(sources: &[SourceInput]) -> Result<Self, String> {
        let mut modules = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            modules.push(SourceModule {
                module_name: source.module_name.clone(),
                ast: parse_module(SourceId(index), &source.text)?,
                is_entry: source.is_entry,
            });
        }
        Ok(Self { modules })
    }
}

fn collect_mojo_source_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_file() => {
            if is_mojo_source(root) {
                out.push(root.to_path_buf());
            }
            Ok(())
        }
        Ok(metadata) if metadata.is_dir() => collect_mojo_source_dir(root, out),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(format!("could not read {}: {source}", root.display())),
    }
}

fn collect_mojo_source_dir(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(root)
        .map_err(|source| format!("could not read {}: {source}", root.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|source| format!("could not read {}: {source}", root.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| format!("could not read {}: {source}", path.display()))?;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolTable {
    pub functions: BTreeMap<String, MojoFunction>,
    pub field_types: BTreeMap<String, MojoType>,
    pub imports: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MojoFunction {
    pub params: Vec<MojoParam>,
    pub return_type: Option<MojoType>,
    pub body: Vec<Stmt>,
    pub owner: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MojoParam {
    pub name: String,
    pub ty: MojoType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MojoType {
    PythonObject,
    None,
    Bool,
    Int,
    Float,
    String,
    OwnedKwargsDictPythonObject,
    Named(String),
    Generic { name: String, args: Vec<MojoType> },
    Unknown(String),
}

impl MojoType {
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let normalized = normalize_mojo_type(raw);
        match normalized.as_str() {
            "PythonObject" => Self::PythonObject,
            "None" => Self::None,
            "Bool" => Self::Bool,
            "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt" | "UInt8" | "UInt16"
            | "UInt32" | "UInt64" => Self::Int,
            "Float16" | "Float32" | "Float64" => Self::Float,
            "String" | "StringSlice" => Self::String,
            _ if normalized.starts_with("OwnedKwargsDict")
                && normalized.contains("PythonObject") =>
            {
                Self::OwnedKwargsDictPythonObject
            }
            _ => parse_generic_type(&normalized).unwrap_or(Self::Named(normalized)),
        }
    }

    #[must_use]
    pub fn normalized(&self) -> String {
        match self {
            Self::PythonObject => "PythonObject".to_string(),
            Self::None => "None".to_string(),
            Self::Bool => "Bool".to_string(),
            Self::Int => "Int".to_string(),
            Self::Float => "Float64".to_string(),
            Self::String => "String".to_string(),
            Self::OwnedKwargsDictPythonObject => "OwnedKwargsDict[PythonObject]".to_string(),
            Self::Named(name) | Self::Unknown(name) => name.clone(),
            Self::Generic { name, args } => {
                let args = args
                    .iter()
                    .map(Self::normalized)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}[{args}]")
            }
        }
    }
}

fn parse_generic_type(text: &str) -> Option<MojoType> {
    let open = find_top_level_char(text, '[')?;
    let close = matching_delimiter(text, open, '[', ']')?;
    if text[close + 1..].trim().is_empty() {
        let name = text[..open].trim();
        if !name.is_empty() {
            return Some(MojoType::Generic {
                name: name.to_string(),
                args: split_top_level_commas(&text[open + 1..close])
                    .into_iter()
                    .map(MojoType::parse)
                    .collect(),
            });
        }
    }
    None
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PythonBindingSurface {
    pub functions: BTreeMap<String, PythonFunctionSurface>,
    pub classes: BTreeMap<String, PythonClassSurface>,
    pub diagnostics: Vec<BindingDiagnostic>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PythonClassSurface {
    pub init: Option<PythonFunctionSurface>,
    pub methods: BTreeMap<String, PythonFunctionSurface>,
    pub static_methods: BTreeMap<String, PythonFunctionSurface>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PythonFunctionSurface {
    pub signature: PythonSignature,
}

impl PythonFunctionSurface {
    #[must_use]
    pub fn varargs(returns: impl Into<String>) -> Self {
        Self {
            signature: PythonSignature {
                params: Vec::new(),
                returns: PythonType(returns.into()),
                varargs: true,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PythonSignature {
    pub params: Vec<PythonParam>,
    pub returns: PythonType,
    pub varargs: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PythonParam {
    pub name: String,
    pub annotation: PythonType,
    pub keyword_rest: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PythonType(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingDiagnostic {
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedBindings {
    symbols: SymbolTable,
    calls: Vec<BindingCall>,
}

/// Analyze a parsed source graph into a Python-visible binding surface.
///
/// # Errors
///
/// Returns an error when exported binding declarations reference unsupported
/// or unresolved Mojo source facts.
pub fn analyze_python_bindings(graph: &SourceGraph) -> Result<PythonBindingSurface, String> {
    let parsed = collect_binding_facts(graph)?;
    resolve_bindings(&parsed)
}

fn collect_binding_facts(graph: &SourceGraph) -> Result<ParsedBindings, String> {
    let mut functions = BTreeMap::new();
    let mut field_types = BTreeMap::new();
    let mut imports = BTreeMap::new();
    let mut calls = Vec::new();
    let mut saw_entry = false;

    for module in &graph.modules {
        collect_module_defs(&mut functions, module)?;
        collect_module_fields(&mut field_types, module);
        if module.is_entry {
            saw_entry = true;
            collect_imports(&mut imports, &module.ast.imports);
            calls.extend(module.ast.binding_calls.clone());
        }
    }

    if !saw_entry {
        return Err("no Mojo module entry source was available for stub generation".to_string());
    }

    Ok(ParsedBindings {
        symbols: SymbolTable {
            functions,
            field_types,
            imports,
        },
        calls,
    })
}

fn collect_module_defs(
    functions: &mut BTreeMap<String, MojoFunction>,
    module: &SourceModule,
) -> Result<(), String> {
    for function in &module.ast.functions {
        let mojo_function = mojo_function_from_decl(function);
        let local_name = function.owner.as_ref().map_or_else(
            || function.name.clone(),
            |owner| format!("{owner}.{}", function.name),
        );
        if module.is_entry || function.owner.is_some() || module.module_name.is_none() {
            functions.insert(local_name.clone(), mojo_function.clone());
        }
        if let Some(module_name) = &module.module_name {
            functions.insert(
                format!("{}.{}", module_name.as_str(), local_name),
                mojo_function,
            );
        }
    }
    Ok(())
}

fn mojo_function_from_decl(function: &FunctionDecl) -> MojoFunction {
    MojoFunction {
        params: function
            .params
            .iter()
            .map(|param| MojoParam {
                name: param.name.clone(),
                ty: MojoType::parse(&param.ty.text),
            })
            .collect(),
        return_type: function
            .return_type
            .as_ref()
            .map(|return_type| MojoType::parse(&return_type.text)),
        body: function.body.clone(),
        owner: function.owner.clone(),
    }
}

fn collect_module_fields(field_types: &mut BTreeMap<String, MojoType>, module: &SourceModule) {
    for struct_decl in &module.ast.structs {
        for field in &struct_decl.fields {
            insert_field_type(field_types, &field.name, MojoType::parse(&field.ty.text));
            insert_field_type(
                field_types,
                &format!("{}.{}", struct_decl.name, field.name),
                MojoType::parse(&field.ty.text),
            );
        }
    }
}

fn insert_field_type(field_types: &mut BTreeMap<String, MojoType>, name: &str, ty: MojoType) {
    match field_types.get_mut(name) {
        Some(existing) if existing != &ty => {
            *existing = MojoType::Unknown(String::new());
        }
        Some(_) => {}
        None => {
            field_types.insert(name.to_string(), ty);
        }
    }
}

fn collect_imports(imports: &mut BTreeMap<String, String>, decls: &[ImportDecl]) {
    for decl in decls {
        for name in &decl.names {
            imports.insert(
                name.alias.clone(),
                format!("{}.{}", decl.module, name.original),
            );
        }
    }
}

fn resolve_bindings(parsed: &ParsedBindings) -> Result<PythonBindingSurface, String> {
    let mut bindings = PythonBindingSurface::default();
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
                let function = stub_function_from_def(parsed, mojo_def, 0)?;
                bindings.functions.insert(visible_name, function);
            }
            BindingKind::PyFunction | BindingKind::PyCFunction => {
                let name = required_visible_name(call, binding_display_name(call.kind))?;
                require_stub_identifier(&name)?;
                bindings
                    .functions
                    .insert(name, PythonFunctionSurface::varargs("object"));
            }
            BindingKind::Method => {
                let (class_name, method_name) = class_and_visible(call, current_class.as_deref())?;
                let target = required_generic(call, "def_method")?;
                let mojo_def = resolve_def(parsed, &target)?;
                let function = stub_function_from_def(parsed, mojo_def, 1)?;
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
                let function = stub_function_from_def(parsed, mojo_def, 0)?;
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
                    .insert(method_name, PythonFunctionSurface::varargs("object"));
            }
            BindingKind::PyInit => {
                let class_name =
                    class_for_type_call(call, current_class.as_deref(), "def_py_init")?;
                bindings.classes.entry(class_name).or_default().init =
                    Some(PythonFunctionSurface::varargs("None"));
            }
            BindingKind::DefaultInit => {
                let class_name =
                    class_for_type_call(call, current_class.as_deref(), "def_init_defaultable")?;
                bindings.classes.entry(class_name).or_default().init =
                    Some(PythonFunctionSurface {
                        signature: PythonSignature {
                            params: Vec::new(),
                            returns: PythonType("None".to_string()),
                            varargs: false,
                        },
                    });
            }
        }
    }

    Ok(bindings)
}

fn binding_display_name(kind: BindingKind) -> &'static str {
    match kind {
        BindingKind::Function => "def_function",
        BindingKind::PyFunction => "def_py_function",
        BindingKind::PyCFunction => "def_py_c_function",
        BindingKind::AddType => "add_type",
        BindingKind::Method => "def_method",
        BindingKind::StaticMethod => "def_staticmethod",
        BindingKind::PyMethod => "def_py_method",
        BindingKind::PyCMethod => "def_py_c_method",
        BindingKind::PyInit => "def_py_init",
        BindingKind::DefaultInit => "def_init_defaultable",
    }
}

fn required_visible_name(call: &BindingCall, binding: &str) -> Result<String, String> {
    call.visible_name
        .clone()
        .ok_or_else(|| format!("`{binding}` is missing a Python-visible string name"))
}

fn required_generic(call: &BindingCall, binding: &str) -> Result<String, String> {
    call.generic
        .clone()
        .ok_or_else(|| format!("`{binding}` is missing a Mojo target in brackets"))
}

fn resolve_def<'a>(parsed: &'a ParsedBindings, target: &str) -> Result<&'a MojoFunction, String> {
    let target = target.trim();
    if let Some(direct) = parsed.symbols.functions.get(target) {
        return Ok(direct);
    }
    if !target.contains('.')
        && let Some(imported_target) = parsed.symbols.imports.get(target)
        && let Some(imported) = parsed.symbols.functions.get(imported_target)
    {
        return Ok(imported);
    }
    let leaf = target
        .rsplit('.')
        .next()
        .and_then(|name| parsed.symbols.functions.get(name));
    leaf.ok_or_else(|| {
        format!("exported binding target `{target}` does not resolve to a local Mojo `def`")
    })
}

fn class_and_visible(
    call: &BindingCall,
    current_class: Option<&str>,
) -> Result<(String, String), String> {
    let method_name = required_visible_name(call, "method binding")?;
    require_stub_identifier(&method_name)?;
    let class_name = class_for_type_call(call, current_class, "method binding")?;
    Ok((class_name, method_name))
}

fn class_for_type_call(
    call: &BindingCall,
    current_class: Option<&str>,
    binding: &str,
) -> Result<String, String> {
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
    parsed: &ParsedBindings,
    mojo_def: &MojoFunction,
    drop_params: usize,
) -> Result<PythonFunctionSurface, String> {
    let mut params = Vec::new();
    let binding_params = mojo_def.params.iter().skip(drop_params).collect::<Vec<_>>();
    let binding_param_count = binding_params.len();
    for (index, param) in binding_params.into_iter().enumerate() {
        if param.ty == MojoType::OwnedKwargsDictPythonObject {
            if index + 1 != binding_param_count {
                return Err(format!(
                    "keyword dict parameter `{}` must be the trailing Python binding argument",
                    param.name
                ));
            }
            params.push(PythonParam {
                name: "kwargs".to_string(),
                annotation: PythonType("object".to_string()),
                keyword_rest: true,
            });
            continue;
        }
        require_stub_identifier(&param.name)?;
        let annotation = python_type_for_mojo(&param.ty).ok_or_else(|| {
            format!(
                "unsupported Python binding parameter type `{}`",
                param.ty.normalized()
            )
        })?;
        params.push(PythonParam {
            name: param.name.clone(),
            annotation: PythonType(annotation),
            keyword_rest: false,
        });
    }
    let returns = match &mojo_def.return_type {
        Some(ty) => python_type_for_mojo_return(parsed, mojo_def, ty).ok_or_else(|| {
            format!(
                "unsupported Python binding return type `{}`",
                ty.normalized()
            )
        })?,
        None => "None".to_string(),
    };
    Ok(PythonFunctionSurface {
        signature: PythonSignature {
            params,
            returns: PythonType(returns),
            varargs: false,
        },
    })
}

fn python_type_for_mojo(ty: &MojoType) -> Option<String> {
    match ty {
        MojoType::PythonObject => Some("object".to_string()),
        MojoType::Bool => Some("bool".to_string()),
        MojoType::Int => Some("int".to_string()),
        MojoType::Float => Some("float".to_string()),
        MojoType::String => Some("str".to_string()),
        _ => None,
    }
}

fn python_type_for_mojo_return(
    parsed: &ParsedBindings,
    mojo_def: &MojoFunction,
    ty: &MojoType,
) -> Option<String> {
    if ty == &MojoType::None {
        Some("None".to_string())
    } else if ty == &MojoType::PythonObject {
        Some(infer_python_object_return(parsed, mojo_def).unwrap_or_else(|| "object".to_string()))
    } else {
        python_type_for_mojo(ty)
    }
}

fn infer_python_object_return(parsed: &ParsedBindings, mojo_def: &MojoFunction) -> Option<String> {
    infer_python_object_return_inner(parsed, mojo_def, &mut Vec::new())
}

fn infer_python_object_return_inner(
    parsed: &ParsedBindings,
    mojo_def: &MojoFunction,
    stack: &mut Vec<String>,
) -> Option<String> {
    let mut types = Vec::new();
    for expression in return_expressions(mojo_def) {
        let inferred = infer_python_object_return_expression(parsed, mojo_def, &expression, stack)?;
        push_unique_type(&mut types, inferred);
    }
    (!types.is_empty()).then(|| types.join(" | "))
}

fn return_expressions(mojo_def: &MojoFunction) -> Vec<String> {
    mojo_def
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Return { expr, .. } => Some(expr.text.clone()),
            _ => None,
        })
        .collect()
}

fn push_unique_type(types: &mut Vec<String>, ty: String) {
    if !types.iter().any(|existing| existing == &ty) {
        types.push(ty);
    }
}

fn infer_python_object_return_expression(
    parsed: &ParsedBindings,
    mojo_def: &MojoFunction,
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
    let return_type = target.return_type.as_ref()?;
    if return_type == &MojoType::PythonObject {
        stack.push(call);
        let inferred = infer_python_object_return_inner(parsed, target, stack);
        let _ = stack.pop();
        inferred
    } else {
        python_type_for_mojo(return_type)
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
    parsed: &ParsedBindings,
    mojo_def: &MojoFunction,
    expression: &str,
) -> Option<String> {
    let expression = strip_mojo_move(expression);
    if valid_python_identifier(expression)
        && let Some(mojo_type) = infer_local_variable_mojo_type(parsed, mojo_def, expression)
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
    parsed: &ParsedBindings,
    mojo_def: &MojoFunction,
    variable: &str,
) -> Option<MojoType> {
    for stmt in &mojo_def.body {
        let Stmt::Let {
            name,
            explicit_type,
            value,
            ..
        } = stmt
        else {
            continue;
        };
        if name != variable {
            continue;
        }
        if let Some(explicit_type) = explicit_type {
            return Some(MojoType::parse(&explicit_type.text));
        }
        if let Some(inferred) = infer_mojo_expression_type(parsed, &value.text) {
            return Some(inferred);
        }
    }
    None
}

fn infer_python_value_type(
    parsed: &ParsedBindings,
    mojo_def: &MojoFunction,
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
        && let Some(ty) = infer_local_variable_mojo_type(parsed, mojo_def, expression)
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

fn infer_mojo_expression_type(parsed: &ParsedBindings, expression: &str) -> Option<MojoType> {
    let call = call_name(strip_mojo_move(expression))?;
    infer_mojo_call_return_type(parsed, &call)
}

fn infer_mojo_call_return_type(parsed: &ParsedBindings, call: &str) -> Option<MojoType> {
    resolve_inferred_def(parsed, call.trim()).and_then(|mojo_def| mojo_def.return_type.clone())
}

fn resolve_inferred_def<'a>(parsed: &'a ParsedBindings, call: &str) -> Option<&'a MojoFunction> {
    if let Some(direct) = parsed.symbols.functions.get(call) {
        return Some(direct);
    }
    let leaf = call.rsplit('.').next().unwrap_or(call).trim();
    let mut match_def: Option<&MojoFunction> = None;
    for (name, mojo_def) in &parsed.symbols.functions {
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

fn python_alloc_type_for_mojo(ty: &MojoType) -> Option<String> {
    let normalized = ty.normalized();
    if normalized.contains('[') || python_type_for_mojo(ty).is_some() {
        return None;
    }
    let leaf = normalized.rsplit('.').next().unwrap_or(&normalized);
    valid_python_identifier(leaf).then(|| leaf.to_string())
}

fn single_bound_class(parsed: &ParsedBindings) -> Option<String> {
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
    parsed: &ParsedBindings,
    mojo_def: &MojoFunction,
    expression: &str,
) -> Option<String> {
    if let Some(owner) = &mojo_def.owner
        && let Some(python_type) = infer_owner_field_access_python_type(parsed, owner, expression)
    {
        return Some(python_type);
    }
    for (field, ty) in &parsed.symbols.field_types {
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
    parsed: &ParsedBindings,
    owner: &str,
    expression: &str,
) -> Option<String> {
    let prefix = format!("{owner}.");
    for (field, ty) in &parsed.symbols.field_types {
        let Some(field) = field.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(python_type) = infer_field_python_type(expression, field, ty) {
            return Some(python_type);
        }
    }
    None
}

fn infer_field_python_type(expression: &str, field: &str, ty: &MojoType) -> Option<String> {
    let normalized = ty.normalized();
    if normalized.is_empty() {
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
        list_item_type(ty).unwrap_or_else(|| ty.clone())
    } else {
        ty.clone()
    };
    python_type_for_mojo(&mojo_type)
}

fn list_item_type(ty: &MojoType) -> Option<MojoType> {
    match ty {
        MojoType::Generic { name, args } if name == "List" && args.len() == 1 => {
            args.first().cloned()
        }
        MojoType::Named(text) | MojoType::Unknown(text) => text
            .strip_prefix("List[")?
            .strip_suffix(']')
            .map(str::trim)
            .map(MojoType::parse),
        _ => None,
    }
}

fn require_stub_identifier(value: &str) -> Result<(), String> {
    if valid_python_identifier(value) {
        Ok(())
    } else {
        Err(format!(
            "`{value}` is not a supported Python stub identifier"
        ))
    }
}

#[allow(dead_code)]
fn clean_stub_identifier(raw: &str) -> Result<String, String> {
    clean_identifier(raw)
}

#[allow(dead_code)]
fn strip_line_comment(line: &str) -> &str {
    strip_comment(line)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use crate::{ModuleName, SourceGraph, SourceInput, analyze_python_bindings, load_source_graph};
    use std::fs;

    fn surface_from_source(source: &str) -> crate::PythonBindingSurface {
        let graph = SourceGraph::parse(&[SourceInput {
            module_name: None,
            text: source.to_string(),
            is_entry: true,
        }])
        .unwrap();
        analyze_python_bindings(&graph).unwrap()
    }

    #[test]
    fn analyzes_classes_methods_staticmethods_and_kwargs() {
        let surface = surface_from_source(
            r#"
from std.collections import OwnedKwargsDict
from std.python import PythonObject

def duration(hours: PythonObject, kwargs: OwnedKwargsDict[PythonObject]) raises -> PythonObject:
    return hours

struct Timer:
    @staticmethod
    def is_valid(value: PythonObject) raises -> PythonObject:
        return value

def PyInit__native() -> PythonObject:
    module.def_function[duration]("duration")
    _ = module.add_type[Timer]("Timer").def_init_defaultable[Timer]().def_staticmethod[Timer.is_valid]("is_valid")
"#,
        );

        let duration = surface.functions.get("duration").unwrap();
        assert_eq!(duration.signature.params[1].name, "kwargs");
        assert!(duration.signature.params[1].keyword_rest);
        assert!(surface.classes["Timer"].init.is_some());
        assert!(
            surface.classes["Timer"]
                .static_methods
                .contains_key("is_valid")
        );
    }

    #[test]
    fn narrows_python_object_wrappers_and_unions() {
        let surface = surface_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder
from std.collections import List

struct Array:
    var dtype_code: Int
    var shape: List[Int]

    @staticmethod
    def shape_at(py_self: PythonObject, index_obj: PythonObject) raises -> PythonObject:
        var self_ptr = py_self.downcast_value_ptr[Self]()
        return PythonObject(self_ptr[].shape[0])

    @staticmethod
    def scalar(py_self: PythonObject) raises -> PythonObject:
        if True:
            return PythonObject(get_bool())
        return PythonObject(get_f64())

def get_bool() -> Bool:
    return True

def get_f64() -> Float64:
    return 1.0

def make_array() raises -> Array:
    pass

def empty() raises -> PythonObject:
    var result = make_array()
    return PythonObject(alloc=result^)

def PyInit__native() -> PythonObject:
    _ = module.add_type[Array]("Array").def_method[Array.shape_at]("shape_at").def_method[Array.scalar]("scalar")
    module.def_function[empty]("empty")
"#,
        );

        assert_eq!(surface.functions["empty"].signature.returns.0, "Array");
        assert_eq!(
            surface.classes["Array"].methods["shape_at"]
                .signature
                .returns
                .0,
            "int"
        );
        assert_eq!(
            surface.classes["Array"].methods["scalar"]
                .signature
                .returns
                .0,
            "bool | float"
        );
    }

    #[test]
    fn resolves_entry_import_aliases_from_source_graph() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from create import imported_fun as aliased_fun

def PyInit__native() -> PythonObject:
    module.def_function[aliased_fun]("aliased_fun")
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/create.mojo"),
            r#"
def imported_fun(value: PythonObject) raises -> PythonObject:
    return value
"#,
        )
        .unwrap();

        let graph = load_source_graph(
            &temp.path().join("src"),
            &[],
            &temp.path().join("src/lib.mojo"),
        )
        .unwrap();
        assert_eq!(
            graph.modules[0].module_name,
            Some(ModuleName("create".to_string()))
        );
        let surface = analyze_python_bindings(&graph).unwrap();
        assert!(surface.functions.contains_key("aliased_fun"));
    }

    #[test]
    fn unresolved_exported_target_is_an_error() {
        let graph = SourceGraph::parse(&[SourceInput {
            module_name: None,
            text: r#"
def PyInit__native() -> PythonObject:
    module.def_function[missing]("missing")
"#
            .to_string(),
            is_entry: true,
        }])
        .unwrap();
        let error = analyze_python_bindings(&graph).unwrap_err();
        assert!(error.contains("missing"));
        assert!(error.contains("does not resolve"));
    }
}
