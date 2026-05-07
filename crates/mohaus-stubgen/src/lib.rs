use mohaus_mojo_bindings::{
    PythonBindingSurface, PythonClassSurface, PythonFunctionSurface, PythonParam, PythonSignature,
};

/// Render a Python `.pyi` stub from a Mojo Python binding surface.
#[must_use]
pub fn render_pyi(bindings: &PythonBindingSurface) -> String {
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
        let wrote_class_item = render_class_items(&mut text, class);
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

fn render_class_items(text: &mut String, class: &PythonClassSurface) -> bool {
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
    wrote_class_item
}

fn render_function(
    prefix: &str,
    name: &str,
    function: &PythonFunctionSurface,
    indent: usize,
    include_self: bool,
) -> String {
    let mut params = Vec::new();
    if include_self {
        params.push("self".to_string());
    }
    if function.signature.varargs {
        params.push("*args: object".to_string());
        params.push("**kwargs: object".to_string());
    } else {
        params.extend(function.signature.params.iter().map(render_param));
    }
    format!(
        "{}{prefix} {name}({}) -> {}: ...\n",
        " ".repeat(indent),
        params.join(", "),
        function.signature.returns.0
    )
}

fn render_param(param: &PythonParam) -> String {
    if param.keyword_rest {
        format!("**{}: {}", param.name, param.annotation.0)
    } else {
        format!("{}: {}", param.name, param.annotation.0)
    }
}

#[allow(dead_code)]
fn signature_returns(signature: &PythonSignature) -> &str {
    &signature.returns.0
}

#[cfg(test)]
mod tests {
    use mohaus_mojo_bindings::{
        PythonBindingSurface, PythonClassSurface, PythonFunctionSurface, PythonParam,
        PythonSignature, PythonType,
    };
    use std::collections::BTreeMap;

    use crate::render_pyi;

    #[test]
    fn renders_empty_surface_as_ellipsis() {
        assert_eq!(render_pyi(&PythonBindingSurface::default()), "...\n");
    }

    #[test]
    fn renders_functions_classes_methods_staticmethods_and_kwargs() {
        let mut surface = PythonBindingSurface::default();
        surface.functions.insert(
            "duration".to_string(),
            PythonFunctionSurface {
                signature: PythonSignature {
                    params: vec![
                        PythonParam {
                            name: "hours".to_string(),
                            annotation: PythonType("object".to_string()),
                            keyword_rest: false,
                        },
                        PythonParam {
                            name: "kwargs".to_string(),
                            annotation: PythonType("object".to_string()),
                            keyword_rest: true,
                        },
                    ],
                    returns: PythonType("object".to_string()),
                    varargs: false,
                },
            },
        );
        surface.classes.insert(
            "Timer".to_string(),
            PythonClassSurface {
                init: Some(PythonFunctionSurface {
                    signature: PythonSignature {
                        params: Vec::new(),
                        returns: PythonType("None".to_string()),
                        varargs: false,
                    },
                }),
                methods: BTreeMap::new(),
                static_methods: BTreeMap::from([(
                    "is_valid".to_string(),
                    PythonFunctionSurface {
                        signature: PythonSignature {
                            params: vec![PythonParam {
                                name: "value".to_string(),
                                annotation: PythonType("bool".to_string()),
                                keyword_rest: false,
                            }],
                            returns: PythonType("bool".to_string()),
                            varargs: false,
                        },
                    },
                )]),
            },
        );

        assert_eq!(
            render_pyi(&surface),
            concat!(
                "def duration(hours: object, **kwargs: object) -> object: ...\n\n",
                "class Timer:\n",
                "  def __init__(self) -> None: ...\n",
                "  @staticmethod\n",
                "  def is_valid(value: bool) -> bool: ...\n",
            )
        );
    }
}
