# mohaus-stubgen (Mojo parity port)
#
# Mirrors the source-level extractor in `crates/mohaus-core/src/stub.rs`.
# It reads Mojo binding declarations and local `def` headers, then renders
# declaration-accurate `.pyi` text without evaluating Mojo code.

from std.collections import List


@fieldwise_init
struct ParsedSource(Copyable, Movable):
    var defs: List[String]
    var calls: List[String]


def parse_binding_source(source: String) raises -> ParsedSource:
    var lines = List[String]()
    for line in source.split("\n"):
        lines.append(String(line))

    var defs = List[String]()
    var binding_source = String("")
    var current_struct = String("")
    var struct_indent = -1
    var index = 0

    while index < len(lines):
        var original = String(lines[index])
        var line = _strip_comment(original)
        var trimmed = String(line.strip())
        var indent = _indentation(original)

        if trimmed.byte_length() > 0 and struct_indent >= 0 and indent <= struct_indent:
            current_struct = String("")
            struct_indent = -1

        var parsed_struct = _parse_struct_name(trimmed)
        if parsed_struct.byte_length() > 0:
            current_struct = parsed_struct
            struct_indent = indent

        if _starts_def(trimmed):
            var collected = _collect_def_header(lines, index)
            var record = _parse_def_header(collected[0])
            defs.append(record)
            if current_struct.byte_length() > 0:
                defs.append(
                    _def_record(
                        current_struct + "." + _def_name(record),
                        _def_params(record),
                        _def_return(record),
                        _def_has_return(record),
                    )
                )
            index = collected[1]
            continue

        binding_source = binding_source + line + "\n"
        index += 1

    return ParsedSource(defs^, _parse_binding_calls(binding_source))


def render_stub_text(source: String) raises -> String:
    return render_parsed_stub(parse_binding_source(source))


def render_parsed_stub(parsed: ParsedSource) raises -> String:
    return _render_resolved(parsed)


def _binding_names() -> List[String]:
    var names = List[String]()
    names.append("def_init_defaultable\tdefault_init")
    names.append("def_py_c_function\tpy_c_function")
    names.append("def_py_c_method\tpy_c_method")
    names.append("def_py_function\tpy_function")
    names.append("def_py_method\tpy_method")
    names.append("def_staticmethod\tstaticmethod")
    names.append("def_py_init\tpy_init")
    names.append("def_function\tfunction")
    names.append("def_method\tmethod")
    names.append("add_type\tadd_type")
    return names^


def _parse_binding_calls(text: String) raises -> List[String]:
    var positioned = List[String]()
    var names = _binding_names()
    for entry in names:
        var parts = _tab_fields(entry)
        var name = parts[0]
        var kind = parts[1]
        var offset = 0
        while offset < text.byte_length():
            var found = _find_from(text, name, offset)
            if found < 0:
                break
            if _is_call_boundary(text, found, name.byte_length()):
                var parsed = _parse_binding_call(text, found, kind, name)
                if parsed[0] == "1":
                    positioned.append(_int_to_string(found) + "\t" + parsed[2])
                    offset = _string_to_int(parsed[1])
                    continue
            offset = found + name.byte_length()

    _sort_positioned_records(positioned)
    var calls = List[String]()
    for record in positioned:
        var tab = _find_from(record, "\t", 0)
        calls.append(_slice(record, tab + 1, record.byte_length()))
    return calls^


def _parse_binding_call(text: String, start: Int, kind: String, name: String) raises -> Tuple[String, String, String]:
    var cursor = start + name.byte_length()
    var generic = String("")
    if cursor < text.byte_length() and _byte_at(text, cursor) == UInt8(91):
        var close = _matching_delimiter(text, cursor, UInt8(91), UInt8(93))
        if close < 0:
            raise Error("binding `", name, "` has unterminated generic args")
        var generic_parts = _split_top_level_commas(_slice(text, cursor + 1, close))
        if len(generic_parts) > 0:
            generic = String(String(generic_parts[0]).strip())
        cursor = close + 1

    cursor = _skip_whitespace(text, cursor)
    if cursor >= text.byte_length() or _byte_at(text, cursor) != UInt8(40):
        return Tuple[String, String, String]("0", _int_to_string(start + name.byte_length()), "")

    var close = _matching_delimiter(text, cursor, UInt8(40), UInt8(41))
    if close < 0:
        raise Error("binding `", name, "` has unterminated call args")
    var visible = _parse_first_string_arg(_slice(text, cursor + 1, close))
    return Tuple[String, String, String]("1", _int_to_string(close + 1), _call_record(kind, generic, visible[0]))


def _call_record(kind: String, generic: String, visible: String) -> String:
    return kind + "\t" + generic + "\t" + visible


def _call_kind(record: String) -> String:
    return _tab_fields(record)[0]


def _call_generic(record: String) -> String:
    return _tab_fields(record)[1]


def _call_visible(record: String) -> String:
    return _tab_fields(record)[2]


def _parse_first_string_arg(args: String) -> Tuple[String, Bool]:
    var parts = _split_top_level_commas(args)
    for raw in parts:
        var parsed = _parse_string_literal(String(String(raw).strip()))
        if parsed[1]:
            return parsed
    return Tuple[String, Bool]("", False)


def _parse_string_literal(text: String) -> Tuple[String, Bool]:
    if text.byte_length() == 0:
        return Tuple[String, Bool]("", False)
    var quote = _byte_at(text, 0)
    if quote != UInt8(34) and quote != UInt8(39):
        return Tuple[String, Bool]("", False)
    var out = List[UInt8]()
    var escaped = False
    var index = 1
    while index < text.byte_length():
        var b = _byte_at(text, index)
        if escaped:
            out.append(b)
            escaped = False
        elif b == UInt8(92):
            escaped = True
        elif b == quote:
            out.append(UInt8(0))
            return Tuple[String, Bool](String(unsafe_from_utf8_ptr=out.unsafe_ptr()), True)
        else:
            out.append(b)
        index += 1
    return Tuple[String, Bool]("", False)


def _collect_def_header(lines: List[String], start: Int) raises -> Tuple[String, Int]:
    var header = String("")
    var index = start
    while index < len(lines):
        if header.byte_length() > 0:
            header = header + " "
        header = header + String(_strip_comment(lines[index]).strip())
        if _top_level_colon_index(header) >= 0:
            return Tuple[String, Int](header, index + 1)
        index += 1
    raise Error("unterminated def header starting at line ", start + 1)


def _parse_def_header(header: String) raises -> String:
    var colon = _top_level_colon_index(header)
    if colon < 0:
        raise Error("def header has no colon")
    var without_colon = String(_slice(header, 0, colon).strip())
    if not without_colon.startswith("def "):
        raise Error("not a def header: ", header)
    var rest = String(_slice(without_colon, 4, without_colon.byte_length()).strip())
    var open = _find_top_level_byte(rest, UInt8(40))
    if open < 0:
        raise Error("def header has no parameter list: ", header)
    var name = _clean_identifier(String(_slice(rest, 0, open).strip()))
    var close = _matching_delimiter(rest, open, UInt8(40), UInt8(41))
    if close < 0:
        raise Error("def `", name, "` has an unterminated parameter list")
    var params = _parse_params(_slice(rest, open + 1, close))
    var suffix = String(_slice(rest, close + 1, rest.byte_length()).strip())
    var return_type = String("")
    var has_return = False
    var arrow = _find_from(suffix, "->", 0)
    if arrow >= 0:
        return_type = String(_slice(suffix, arrow + 2, suffix.byte_length()).strip())
        has_return = return_type.byte_length() > 0
    return _def_record(name, params, return_type, has_return)


def _parse_params(params: String) raises -> String:
    var encoded = String("")
    var parts = _split_top_level_commas(params)
    for part in parts:
        var raw = String(String(part).strip())
        if raw.byte_length() == 0 or raw == "*":
            continue
        raw = _strip_default(raw)
        var colon = _find_top_level_byte(raw, UInt8(58))
        if colon < 0:
            continue
        var name_part = String(_slice(raw, 0, colon).strip())
        var words = name_part.split()
        if len(words) == 0:
            continue
        var name = _clean_identifier(String(words[len(words) - 1]))
        var ty = String(_slice(raw, colon + 1, raw.byte_length()).strip())
        encoded = _join_record(encoded, name + ":" + ty, "|")
    return encoded


def _strip_default(raw: String) -> String:
    var equals = _find_top_level_byte(raw, UInt8(61))
    if equals >= 0:
        return String(_slice(raw, 0, equals).strip())
    return raw


def _def_record(name: String, params: String, return_type: String, has_return: Bool) -> String:
    var has = String("0")
    if has_return:
        has = "1"
    return name + "\t" + params + "\t" + has + "\t" + return_type


def _def_name(record: String) -> String:
    return _tab_fields(record)[0]


def _def_params(record: String) -> String:
    return _tab_fields(record)[1]


def _def_has_return(record: String) -> Bool:
    return _tab_fields(record)[2] == "1"


def _def_return(record: String) -> String:
    return _tab_fields(record)[3]


def _render_resolved(parsed: ParsedSource) raises -> String:
    var functions = List[String]()
    var class_names = List[String]()
    var class_has_init = List[Bool]()
    var class_inits = List[String]()
    var methods = List[String]()
    var static_methods = List[String]()
    var current_class = String("")

    for call in parsed.calls:
        var kind = _call_kind(call)
        if kind == "add_type":
            var class_name = _required_visible_name(call, "add_type")
            _require_stub_identifier(class_name)
            _ = _ensure_class(class_names, class_has_init, class_inits, class_name)
            current_class = class_name
        elif kind == "function":
            var visible = _required_visible_name(call, "def_function")
            _require_stub_identifier(visible)
            var target = _required_generic(call, "def_function")
            functions.append(visible + "\t" + _stub_function_from_def(_resolve_def(parsed, target), 0))
        elif kind == "py_function" or kind == "py_c_function":
            var visible = _required_visible_name(call, kind)
            _require_stub_identifier(visible)
            functions.append(visible + "\t" + _broad_function("object"))
        elif kind == "method":
            var class_and_name = _class_and_visible(call, current_class)
            var target = _required_generic(call, "def_method")
            _ = _ensure_class(class_names, class_has_init, class_inits, class_and_name[0])
            methods.append(
                class_and_name[0]
                + "\t"
                + class_and_name[1]
                + "\t"
                + _stub_function_from_def(_resolve_def(parsed, target), 1)
            )
        elif kind == "staticmethod":
            var class_and_name = _class_and_visible(call, current_class)
            var target = _required_generic(call, "def_staticmethod")
            _ = _ensure_class(class_names, class_has_init, class_inits, class_and_name[0])
            static_methods.append(
                class_and_name[0]
                + "\t"
                + class_and_name[1]
                + "\t"
                + _stub_function_from_def(_resolve_def(parsed, target), 0)
            )
        elif kind == "py_method" or kind == "py_c_method":
            var class_and_name = _class_and_visible(call, current_class)
            _ = _ensure_class(class_names, class_has_init, class_inits, class_and_name[0])
            methods.append(class_and_name[0] + "\t" + class_and_name[1] + "\t" + _broad_function("object"))
        elif kind == "py_init":
            var class_name = _class_for_type_call(call, current_class, "def_py_init")
            var class_index = _ensure_class(class_names, class_has_init, class_inits, class_name)
            class_has_init[class_index] = True
            class_inits[class_index] = _broad_function("None")
        elif kind == "default_init":
            var class_name = _class_for_type_call(call, current_class, "def_init_defaultable")
            var class_index = _ensure_class(class_names, class_has_init, class_inits, class_name)
            class_has_init[class_index] = True
            class_inits[class_index] = "\tNone"

    return _render_bindings(functions, class_names, class_has_init, class_inits, methods, static_methods)


def _stub_function_from_def(def_record: String, drop_params: Int) raises -> String:
    var params = String("")
    var raw_params = _split_byte(_def_params(def_record), UInt8(124))
    for index in range(drop_params, len(raw_params)):
        var pair = String(raw_params[index])
        if pair.byte_length() == 0:
            continue
        var colon = _find_from(pair, ":", 0)
        if colon < 0:
            continue
        var name = _slice(pair, 0, colon)
        var ty = _slice(pair, colon + 1, pair.byte_length())
        var normalized = _normalize_mojo_type(ty)
        if _is_owned_kwargs_dict(normalized):
            if index + 1 != len(raw_params):
                raise Error("keyword dict parameter `", name, "` must be the trailing Python binding argument")
            params = _join_record(params, "**kwargs: object", ", ")
            continue
        _require_stub_identifier(name)
        var annotation = _python_type_for_mojo(normalized)
        if annotation.byte_length() == 0:
            raise Error("unsupported Python binding parameter type `", ty, "`")
        params = _join_record(params, name + ": " + annotation, ", ")

    var returns = String("None")
    if _def_has_return(def_record):
        returns = _python_type_for_mojo_return(_normalize_mojo_type(_def_return(def_record)))
        if returns.byte_length() == 0:
            raise Error("unsupported Python binding return type `", _def_return(def_record), "`")
    return params + "\t" + returns


def _broad_function(returns: String) -> String:
    return "*args: object, **kwargs: object\t" + returns


def _resolve_def(parsed: ParsedSource, target: String) raises -> String:
    var trimmed = String(target.strip())
    for mojo_def in parsed.defs:
        if _def_name(mojo_def) == trimmed:
            return String(mojo_def)
    var leaf = _leaf_after_dot(trimmed)
    for mojo_def in parsed.defs:
        if _def_name(mojo_def) == leaf:
            return String(mojo_def)
    raise Error("exported binding target `", trimmed, "` does not resolve to a local Mojo `def`")


def _class_and_visible(call: String, current_class: String) raises -> Tuple[String, String]:
    var method_name = _required_visible_name(call, "method binding")
    _require_stub_identifier(method_name)
    var class_name = _class_for_type_call(call, current_class, "method binding")
    return Tuple[String, String](class_name, method_name)


def _class_for_type_call(call: String, current_class: String, binding: String) raises -> String:
    var generic = _call_generic(call)
    if generic.byte_length() > 0:
        var class_name = _class_from_generic(generic)
        if class_name.byte_length() > 0:
            _require_stub_identifier(class_name)
            return class_name
        if binding == "def_init_defaultable" or binding == "def_py_init":
            var direct = String(generic.strip())
            if direct.byte_length() > 0:
                _require_stub_identifier(direct)
                return direct
    if current_class.byte_length() > 0:
        return current_class
    raise Error("`", binding, "` is not attached to a discoverable `add_type`")


def _class_from_generic(generic: String) -> String:
    var last_dot = _rfind_byte(generic, UInt8(46))
    if last_dot < 0:
        return String("")
    return _leaf_after_dot(_slice(generic, 0, last_dot))


def _required_visible_name(call: String, binding: String) raises -> String:
    var visible = _call_visible(call)
    if visible.byte_length() > 0:
        return visible
    raise Error("`", binding, "` is missing a Python-visible string name")


def _required_generic(call: String, binding: String) raises -> String:
    var generic = _call_generic(call)
    if generic.byte_length() > 0:
        return generic
    raise Error("`", binding, "` is missing a Mojo target in brackets")


def _ensure_class(mut names: List[String], mut has_init: List[Bool], mut inits: List[String], name: String) -> Int:
    for index in range(len(names)):
        if names[index] == name:
            return index
    names.append(name)
    has_init.append(False)
    inits.append("\tNone")
    return len(names) - 1


def _render_bindings(
    functions: List[String],
    class_names: List[String],
    class_has_init: List[Bool],
    class_inits: List[String],
    methods: List[String],
    static_methods: List[String],
) -> String:
    var text = String("")
    var wrote_item = False

    for entry in functions:
        var fields = _tab_fields(entry)
        text = text + _render_function(fields[0], fields[1] + "\t" + fields[2], 0, False)
        wrote_item = True

    for index in range(len(class_names)):
        var class_name = class_names[index]
        if wrote_item:
            text = text + "\n"
        text = text + "class " + class_name + ":\n"
        var wrote_class_item = False
        if class_has_init[index]:
            text = text + _render_function("__init__", class_inits[index], 2, True)
            wrote_class_item = True
        for method in methods:
            var fields = _tab_fields(method)
            if fields[0] == class_name:
                text = text + _render_function(fields[1], fields[2] + "\t" + fields[3], 2, True)
                wrote_class_item = True
        for method in static_methods:
            var fields = _tab_fields(method)
            if fields[0] == class_name:
                text = text + "  @staticmethod\n"
                text = text + _render_function(fields[1], fields[2] + "\t" + fields[3], 2, False)
                wrote_class_item = True
        if not wrote_class_item:
            text = text + "  ...\n"
        wrote_item = True

    if not wrote_item:
        text = "...\n"
    return text


def _render_function(name: String, function_record: String, indent: Int, include_self: Bool) -> String:
    var fields = _tab_fields(function_record)
    var params = fields[0]
    var returns = fields[1]
    if include_self:
        if params.byte_length() > 0:
            params = "self, " + params
        else:
            params = "self"
    return _spaces(indent) + "def " + name + "(" + params + ") -> " + returns + ": ...\n"


def _normalize_mojo_type(ty: String) -> String:
    var normalized = String(ty.strip())
    if normalized.startswith("mut "):
        normalized = String(_slice(normalized, 4, normalized.byte_length()).strip())
    if normalized.startswith("owned "):
        normalized = String(_slice(normalized, 6, normalized.byte_length()).strip())
    return normalized


def _is_owned_kwargs_dict(ty: String) -> Bool:
    return ty.startswith("OwnedKwargsDict") and _find_from(ty, "PythonObject", 0) >= 0


def _python_type_for_mojo(ty: String) -> String:
    if ty == "PythonObject":
        return String("object")
    if ty == "Bool":
        return String("bool")
    if (
        ty == "Int"
        or ty == "Int8"
        or ty == "Int16"
        or ty == "Int32"
        or ty == "Int64"
        or ty == "UInt"
        or ty == "UInt8"
        or ty == "UInt16"
        or ty == "UInt32"
        or ty == "UInt64"
    ):
        return String("int")
    if ty == "Float16" or ty == "Float32" or ty == "Float64":
        return String("float")
    if ty == "String" or ty == "StringSlice":
        return String("str")
    return String("")


def _python_type_for_mojo_return(ty: String) -> String:
    if ty == "None":
        return String("None")
    return _python_type_for_mojo(ty)


def _parse_struct_name(trimmed: String) -> String:
    if not trimmed.startswith("struct "):
        return String("")
    var rest = _slice(trimmed, 7, trimmed.byte_length())
    var end = 0
    while end < rest.byte_length() and _is_ident_byte(_byte_at(rest, end)):
        end += 1
    var name = _slice(rest, 0, end)
    if _valid_python_identifier(name):
        return name
    return String("")


def _starts_def(trimmed: String) -> Bool:
    return trimmed.startswith("def ") or trimmed.startswith("def `")


def _clean_identifier(raw: String) raises -> String:
    var trimmed = String(raw.strip())
    if trimmed.startswith("`") and trimmed.endswith("`"):
        trimmed = _slice(trimmed, 1, trimmed.byte_length() - 1)
    if _valid_python_identifier(trimmed):
        return trimmed
    raise Error("`", raw, "` is not a supported Python identifier")


def _require_stub_identifier(value: String) raises:
    if not _valid_python_identifier(value):
        raise Error("`", value, "` is not a supported Python stub identifier")


def _valid_python_identifier(value: String) -> Bool:
    if value.byte_length() == 0 or _is_python_keyword(value):
        return False
    var first = _byte_at(value, 0)
    if not (_is_alpha_byte(first) or first == UInt8(95)):
        return False
    for index in range(1, value.byte_length()):
        var b = _byte_at(value, index)
        if not (_is_alpha_byte(b) or _is_digit_byte(b) or b == UInt8(95)):
            return False
    return True


def _is_python_keyword(value: String) -> Bool:
    return (
        value == "False"
        or value == "None"
        or value == "True"
        or value == "and"
        or value == "as"
        or value == "assert"
        or value == "async"
        or value == "await"
        or value == "break"
        or value == "class"
        or value == "continue"
        or value == "def"
        or value == "del"
        or value == "elif"
        or value == "else"
        or value == "except"
        or value == "finally"
        or value == "for"
        or value == "from"
        or value == "global"
        or value == "if"
        or value == "import"
        or value == "in"
        or value == "is"
        or value == "lambda"
        or value == "nonlocal"
        or value == "not"
        or value == "or"
        or value == "pass"
        or value == "raise"
        or value == "return"
        or value == "try"
        or value == "while"
        or value == "with"
        or value == "yield"
    )


def _strip_comment(line: String) -> String:
    var quote = UInt8(0)
    var escaped = False
    for index in range(line.byte_length()):
        var b = _byte_at(line, index)
        if escaped:
            escaped = False
            continue
        if b == UInt8(92):
            escaped = True
            continue
        if quote != UInt8(0):
            if b == quote:
                quote = UInt8(0)
            continue
        if b == UInt8(34) or b == UInt8(39):
            quote = b
        elif b == UInt8(35):
            return _slice(line, 0, index)
    return line


def _indentation(line: String) -> Int:
    var count = 0
    while count < line.byte_length():
        var b = _byte_at(line, count)
        if b != UInt8(32) and b != UInt8(9):
            break
        count += 1
    return count


def _top_level_colon_index(text: String) -> Int:
    return _find_top_level_byte(text, UInt8(58))


def _find_top_level_byte(text: String, needle: UInt8) -> Int:
    var parens = 0
    var brackets = 0
    var braces = 0
    var quote = UInt8(0)
    var escaped = False
    for index in range(text.byte_length()):
        var b = _byte_at(text, index)
        if escaped:
            escaped = False
            continue
        if b == UInt8(92):
            escaped = True
            continue
        if quote != UInt8(0):
            if b == quote:
                quote = UInt8(0)
            continue
        if b == UInt8(34) or b == UInt8(39):
            quote = b
            continue
        if b == needle and parens == 0 and brackets == 0 and braces == 0:
            return index
        if b == UInt8(40):
            parens += 1
        elif b == UInt8(41):
            parens -= 1
        elif b == UInt8(91):
            brackets += 1
        elif b == UInt8(93):
            brackets -= 1
        elif b == UInt8(123):
            braces += 1
        elif b == UInt8(125):
            braces -= 1
    return -1


def _matching_delimiter(text: String, open: Int, start: UInt8, end: UInt8) -> Int:
    var depth = 0
    var quote = UInt8(0)
    var escaped = False
    for index in range(open, text.byte_length()):
        var b = _byte_at(text, index)
        if escaped:
            escaped = False
            continue
        if b == UInt8(92):
            escaped = True
            continue
        if quote != UInt8(0):
            if b == quote:
                quote = UInt8(0)
            continue
        if b == UInt8(34) or b == UInt8(39):
            quote = b
            continue
        if b == start:
            depth += 1
        elif b == end:
            depth -= 1
            if depth == 0:
                return index
    return -1


def _split_top_level_commas(text: String) -> List[String]:
    var parts = List[String]()
    var start = 0
    var parens = 0
    var brackets = 0
    var braces = 0
    var quote = UInt8(0)
    var escaped = False
    for index in range(text.byte_length()):
        var b = _byte_at(text, index)
        if escaped:
            escaped = False
            continue
        if b == UInt8(92):
            escaped = True
            continue
        if quote != UInt8(0):
            if b == quote:
                quote = UInt8(0)
            continue
        if b == UInt8(34) or b == UInt8(39):
            quote = b
            continue
        if b == UInt8(40):
            parens += 1
        elif b == UInt8(41):
            parens -= 1
        elif b == UInt8(91):
            brackets += 1
        elif b == UInt8(93):
            brackets -= 1
        elif b == UInt8(123):
            braces += 1
        elif b == UInt8(125):
            braces -= 1
        elif b == UInt8(44) and parens == 0 and brackets == 0 and braces == 0:
            parts.append(_slice(text, start, index))
            start = index + 1
    parts.append(_slice(text, start, text.byte_length()))
    return parts^


def _split_byte(text: String, separator: UInt8) -> List[String]:
    var parts = List[String]()
    var start = 0
    for index in range(text.byte_length()):
        if _byte_at(text, index) == separator:
            parts.append(_slice(text, start, index))
            start = index + 1
    parts.append(_slice(text, start, text.byte_length()))
    return parts^


def _tab_fields(text: String) -> List[String]:
    return _split_byte(text, UInt8(9))


def _join_record(existing: String, item: String, separator: String) -> String:
    if existing.byte_length() == 0:
        return item
    return existing + separator + item


def _skip_whitespace(text: String, start: Int) -> Int:
    var index = start
    while index < text.byte_length():
        var b = _byte_at(text, index)
        if b != UInt8(32) and b != UInt8(9) and b != UInt8(10) and b != UInt8(13):
            break
        index += 1
    return index


def _find_from(text: String, needle: String, start: Int) -> Int:
    if needle.byte_length() == 0:
        return start
    var max_start = text.byte_length() - needle.byte_length()
    if max_start < start:
        return -1
    for index in range(start, max_start + 1):
        var matched = True
        for offset in range(needle.byte_length()):
            if _byte_at(text, index + offset) != _byte_at(needle, offset):
                matched = False
                break
        if matched:
            return index
    return -1


def _rfind_byte(text: String, needle: UInt8) -> Int:
    var index = text.byte_length() - 1
    while index >= 0:
        if _byte_at(text, index) == needle:
            return index
        index -= 1
    return -1


def _leaf_after_dot(value: String) -> String:
    var dot = _rfind_byte(value, UInt8(46))
    if dot < 0:
        return value
    return _slice(value, dot + 1, value.byte_length())


def _is_call_boundary(text: String, start: Int, length: Int) -> Bool:
    var before_ok = True
    if start > 0:
        before_ok = not _is_ident_byte(_byte_at(text, start - 1))
    var after_ok = True
    var after = start + length
    if after < text.byte_length():
        var b = _byte_at(text, after)
        after_ok = b == UInt8(91) or b == UInt8(40)
    return before_ok and after_ok


def _sort_positioned_records(mut records: List[String]):
    var n = len(records)
    for i in range(1, n):
        var j = i
        while j > 0 and _record_position(records[j - 1]) > _record_position(records[j]):
            var tmp = String(records[j - 1])
            records[j - 1] = String(records[j])
            records[j] = tmp
            j -= 1


def _record_position(record: String) -> Int:
    var tab = _find_from(record, "\t", 0)
    if tab < 0:
        return 0
    return _string_to_int(_slice(record, 0, tab))


def _string_to_int(text: String) -> Int:
    var value = 0
    for index in range(text.byte_length()):
        var b = _byte_at(text, index)
        if not _is_digit_byte(b):
            break
        value = value * 10 + Int(b - UInt8(48))
    return value


def _int_to_string(value: Int) -> String:
    if value == 0:
        return String("0")
    var digits = List[UInt8]()
    var current = value
    while current > 0:
        digits.append(UInt8(48 + current % 10))
        current = current // 10
    var out = List[UInt8]()
    var index = len(digits) - 1
    while index >= 0:
        out.append(digits[index])
        index -= 1
    out.append(UInt8(0))
    return String(unsafe_from_utf8_ptr=out.unsafe_ptr())


def _byte_at(text: String, index: Int) -> UInt8:
    return text.as_bytes()[index]


def _slice(text: String, start: Int, end: Int) -> String:
    var out = List[UInt8]()
    for index in range(start, end):
        out.append(_byte_at(text, index))
    out.append(UInt8(0))
    return String(unsafe_from_utf8_ptr=out.unsafe_ptr())


def _spaces(count: Int) -> String:
    var out = List[UInt8]()
    for _ in range(count):
        out.append(UInt8(32))
    out.append(UInt8(0))
    return String(unsafe_from_utf8_ptr=out.unsafe_ptr())


def _is_ident_byte(b: UInt8) -> Bool:
    return _is_alpha_byte(b) or _is_digit_byte(b) or b == UInt8(95)


def _is_alpha_byte(b: UInt8) -> Bool:
    return (b >= UInt8(97) and b <= UInt8(122)) or (b >= UInt8(65) and b <= UInt8(90))


def _is_digit_byte(b: UInt8) -> Bool:
    return b >= UInt8(48) and b <= UInt8(57)
