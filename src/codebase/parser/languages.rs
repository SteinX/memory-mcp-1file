use crate::types::symbol::{CodeRelationType, SymbolType};
use crate::types::Language;

pub trait LanguageSupport: Send + Sync {
    fn get_language(&self) -> tree_sitter::Language;
    fn get_definition_query(&self) -> &str;
    fn get_reference_query(&self) -> &str;

    fn map_symbol_type(&self, kind: &str) -> SymbolType;
    fn map_relation_type(&self, kind: &str) -> CodeRelationType;

    fn extract_symbol_name(&self, _kind: &str, raw_name: &str) -> String {
        raw_name.to_string()
    }

    fn extract_signature(&self, parent_node: &tree_sitter::Node, content: &[u8]) -> Option<String> {
        let text = parent_node.utf8_text(content).ok()?;
        let sig = extract_until_body_start(text);
        if sig.is_empty() {
            None
        } else {
            Some(sig.chars().take(500).collect())
        }
    }

    fn extract_reference_name(&self, _kind: &str, raw_name: &str) -> String {
        raw_name.to_string()
    }
}


fn clean_c_include_name(name: &str) -> String {
    name.trim_matches(|ch| ch == '<' || ch == '>' || ch == '"')
        .to_string()
}

fn extract_until_body_start(text: &str) -> String {
    let mut depth = 0;
    let mut result = String::new();

    for ch in text.chars() {
        match ch {
            '{' | '[' if depth == 0 => break,
            '(' => {
                depth += 1;
                result.push(ch);
            }
            ')' => {
                depth -= 1;
                result.push(ch);
            }
            '\n' if depth == 0 => {
                result.push(' ');
            }
            _ => result.push(ch),
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub struct RustSupport;
impl LanguageSupport for RustSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (function_item name: (identifier) @function)
        (function_signature_item name: (identifier) @function)
        (struct_item name: (type_identifier) @struct)
        (enum_item name: (type_identifier) @enum)
        (mod_item name: (identifier) @module)
        (trait_item name: (type_identifier) @trait)
        (impl_item type: (type_identifier) @impl)
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (call_expression function: (identifier) @call)
        (call_expression function: (field_expression field: (field_identifier) @method_call))
        (call_expression function: (scoped_identifier name: (identifier) @call))
        (use_declaration argument: (scoped_identifier name: (identifier) @import))
        (impl_item trait: (type_identifier) @implements)
        "#
    }

    fn extract_symbol_name(&self, kind: &str, raw_name: &str) -> String {
        match kind {
            "method" if raw_name.trim_start().starts_with("init") => "init".to_string(),
            _ => raw_name.to_string(),
        }
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "function" => SymbolType::Function,
            "struct" => SymbolType::Struct,
            "enum" => SymbolType::Enum,
            "module" => SymbolType::Module,
            "trait" => SymbolType::Trait,
            "impl" => SymbolType::Class, // Rust impls are roughly classes
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct PythonSupport;
impl LanguageSupport for PythonSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (function_definition name: (identifier) @function)
        (class_definition name: (identifier) @class)
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (call function: (identifier) @call)
        (call function: (attribute attribute: (identifier) @method_call))
        (import_statement name: (dotted_name (identifier) @import))
        (import_from_statement name: (dotted_name (identifier) @import))
        (class_definition superclasses: (argument_list (identifier) @extends))
        "#
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "function" => SymbolType::Function,
            "class" => SymbolType::Class,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct TypeScriptSupport;
impl LanguageSupport for TypeScriptSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (function_declaration name: (identifier) @function)
        (class_declaration name: (type_identifier) @class)
        (interface_declaration name: (type_identifier) @interface)
        (method_definition name: (property_identifier) @method)
        (export_statement (function_declaration name: (identifier) @function))
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (call_expression function: (identifier) @call)
        (call_expression function: (member_expression property: (property_identifier) @method_call))
        (import_statement source: (string (string_fragment) @import))
        (class_heritage (extends_clause value: (identifier) @extends))
        (class_heritage (implements_clause (type) @implements))
        "#
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "function" => SymbolType::Function,
            "class" => SymbolType::Class,
            "interface" => SymbolType::Interface,
            "method" => SymbolType::Method,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct JavaScriptSupport;
impl LanguageSupport for JavaScriptSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_javascript::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (function_declaration name: (identifier) @function)
        (class_declaration name: (identifier) @class)
        (method_definition name: (property_identifier) @method)
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (call_expression function: (identifier) @call)
        (call_expression function: (member_expression property: (property_identifier) @method_call))
        (import_statement source: (string (string_fragment) @import))
        (class_heritage (identifier) @extends)
        "#
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "function" => SymbolType::Function,
            "class" => SymbolType::Class,
            "method" => SymbolType::Method,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct GoSupport;
impl LanguageSupport for GoSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_go::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (function_declaration name: (identifier) @function)
        (method_declaration name: (field_identifier) @method)
        (type_declaration (type_spec name: (type_identifier) @class))
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (call_expression function: (identifier) @call)
        (call_expression function: (selector_expression field: (field_identifier) @method_call))
        (import_spec path: (string_literal) @import)
        "#
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "function" => SymbolType::Function,
            "method" => SymbolType::Method,
            "class" => SymbolType::Class, // Go structs/interfaces
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct JavaSupport;
impl LanguageSupport for JavaSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_java::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (class_declaration name: (identifier) @class)
        (method_declaration name: (identifier) @method)
        (interface_declaration name: (identifier) @interface)
        (enum_declaration name: (identifier) @enum)
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (method_invocation name: (identifier) @call)
        (import_declaration name: (scoped_identifier) @import)
        (class_declaration superclass: (superclass (type_identifier) @extends))
        (class_declaration interfaces: (super_interfaces (type_list (type_identifier) @implements)))
        "#
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "class" => SymbolType::Class,
            "method" => SymbolType::Method,
            "interface" => SymbolType::Interface,
            "enum" => SymbolType::Enum,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}


pub struct CSupport;
impl LanguageSupport for CSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_c::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (function_definition
          declarator: (function_declarator
            declarator: (identifier) @function))
        (function_definition
          declarator: (pointer_declarator
            declarator: (function_declarator
              declarator: (identifier) @function)))
        (struct_specifier name: (type_identifier) @struct)
        (enum_specifier name: (type_identifier) @enum)
        (type_definition declarator: (type_identifier) @struct)
        (type_definition
          declarator: (pointer_declarator
            declarator: (type_identifier) @struct))
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (call_expression
          function: (identifier) @call)
        (preproc_include path: (system_lib_string) @system_import)
        (preproc_include path: (string_literal (string_content) @import))
        "#
    }



    fn extract_reference_name(&self, kind: &str, raw_name: &str) -> String {
        match kind {
            "import" | "system_import" => clean_c_include_name(raw_name),
            _ => raw_name.to_string(),
        }
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "function" => SymbolType::Function,
            "struct" => SymbolType::Struct,
            "enum" => SymbolType::Enum,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct CppSupport;
impl LanguageSupport for CppSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_cpp::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (namespace_definition name: (namespace_identifier) @module)
        (function_definition
          declarator: (function_declarator
            declarator: (identifier) @function))
        (function_definition
          declarator: (qualified_identifier
            name: (identifier) @function))
        (function_definition
          declarator: (function_declarator
            declarator: (qualified_identifier
              name: (identifier) @method)))
        (function_definition
          declarator: (function_declarator
            declarator: (field_identifier) @method))
        (function_definition
          declarator: (pointer_declarator
            declarator: (function_declarator
              declarator: (identifier) @function)))
        (class_specifier name: (type_identifier) @class)
        (struct_specifier name: (type_identifier) @struct)
        (enum_specifier name: (type_identifier) @enum)
        (declaration
          declarator: (function_declarator
            declarator: (identifier) @method))
        (declaration
          declarator: (function_declarator
            declarator: (field_identifier) @method))
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (call_expression
          function: (identifier) @call)
        (call_expression
          function: (field_expression
            field: (field_identifier) @method_call))
        (call_expression
          function: (qualified_identifier
            name: (identifier) @call))
        (preproc_include path: (system_lib_string) @system_import)
        (preproc_include path: (string_literal (string_content) @import))
        "#
    }


    fn extract_reference_name(&self, kind: &str, raw_name: &str) -> String {
        match kind {
            "import" | "system_import" => clean_c_include_name(raw_name),
            _ => raw_name.to_string(),
        }
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "module" => SymbolType::Module,
            "class" => SymbolType::Class,
            "struct" => SymbolType::Struct,
            "method" => SymbolType::Method,
            "function" => SymbolType::Function,
            "enum" => SymbolType::Enum,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct SwiftSupport;
impl LanguageSupport for SwiftSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_swift::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (protocol_declaration name: (type_identifier) @interface)
        (class_declaration declaration_kind: "class" name: (type_identifier) @class)
        (class_declaration declaration_kind: "struct" name: (type_identifier) @struct)
        (class_declaration declaration_kind: "enum" name: (type_identifier) @enum)
        (source_file (function_declaration name: (simple_identifier) @function))
        (class_body (function_declaration name: (simple_identifier) @method))
        (enum_class_body (function_declaration name: (simple_identifier) @method))
        (protocol_body (protocol_function_declaration name: (simple_identifier) @method))
        (class_body (init_declaration name: "init" @method))
        (enum_class_body (init_declaration name: "init" @method))
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (import_declaration (identifier) @import)
        (call_expression
          (simple_identifier) @call
          (call_suffix))
        (call_expression
          (navigation_expression
            (navigation_suffix suffix: (simple_identifier) @method_call))
          (call_suffix))
        "#
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "class" => SymbolType::Class,
            "struct" => SymbolType::Struct,
            "enum" => SymbolType::Enum,
            "interface" => SymbolType::Interface,
            "method" => SymbolType::Method,
            "function" => SymbolType::Function,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct DartSupport;
impl LanguageSupport for DartSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_dart_orchard::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (class_definition name: (identifier) @class)
        (program (function_signature name: (identifier) @function))
        (declaration (function_signature name: (identifier) @function))
        (method_signature (function_signature name: (identifier) @method))
        (method_signature (getter_signature (identifier) @method))
        (method_signature (setter_signature (identifier) @method))
        (method_signature (constructor_signature name: (identifier) @method))
        (enum_declaration name: (identifier) @enum)
        (mixin_declaration (identifier) @class)
        (extension_declaration name: (identifier) @class)
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        ; Function calls: print("hello"), someFunction(42), setState(() {})
        (((identifier) @call)
         . (selector . (argument_part)))

        ; Method calls: client.fetchData(url), Navigator.of(context)
        ((selector
          (unconditional_assignable_selector "." (identifier) @method_call))
         . (selector (argument_part)))

        ; Conditional method calls: widget?.build(context)
        ((selector
          (conditional_assignable_selector "?." (identifier) @method_call))
         . (selector (argument_part)))

        ; Cascade calls: list..add(1)
        (cascade_section
          (cascade_selector (identifier) @method_call)
          (argument_part))

        ; Imports
        (import_or_export (library_import (import_specification (configurable_uri (uri (string_literal) @import)))))

        ; Extends: class Foo extends Bar
        (class_definition
          superclass: (superclass (type_identifier) @extends))

        ; Implements: class Foo implements Bar, Baz
        (class_definition
          interfaces: (interfaces (type_identifier) @implements))

        ; With (mixins): class Foo extends Bar with Mixin1
        (superclass (mixins (type_identifier) @implements))
        "#
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "class" => SymbolType::Class,
            "method" => SymbolType::Method,
            "function" => SymbolType::Function,
            "enum" => SymbolType::Enum,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct KotlinSupport;
impl LanguageSupport for KotlinSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_kotlin_ng::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (package_header (qualified_identifier) @module)
        (class_declaration name: (identifier) @class)
        (class_declaration "interface" name: (identifier) @interface)
        (object_declaration name: (identifier) @class)
        (companion_object) @class
        (companion_object name: (identifier) @class)
        (class_body (function_declaration name: (identifier) @method))
        (function_declaration name: (identifier) @function)
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (import (qualified_identifier) @import)
        (call_expression (identifier) @call)
        (call_expression (navigation_expression (identifier) @method_call))
        "#
    }

    fn extract_symbol_name(&self, kind: &str, raw_name: &str) -> String {
        match kind {
            "class" if raw_name.starts_with("companion object") => "Companion".to_string(),
            _ => raw_name.to_string(),
        }
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "module" => SymbolType::Module,
            "interface" => SymbolType::Interface,
            "class" => SymbolType::Class,
            "method" => SymbolType::Method,
            "function" => SymbolType::Function,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub struct ObjectiveCSupport;
impl LanguageSupport for ObjectiveCSupport {
    fn get_language(&self) -> tree_sitter::Language {
        tree_sitter_objc::LANGUAGE.into()
    }

    fn get_definition_query(&self) -> &str {
        r#"
        (protocol_declaration (identifier) @interface)
        (class_interface (identifier) @class)
        (method_declaration (identifier) @method)
        (method_definition (identifier) @method)
        "#
    }

    fn get_reference_query(&self) -> &str {
        r#"
        (preproc_include path: (system_lib_string) @system_import)
        (preproc_include path: (string_literal (string_content) @import))
        (call_expression function: (identifier) @call)
        (message_expression method: (identifier) @method_call)
        "#
    }

    fn extract_reference_name(&self, kind: &str, raw_name: &str) -> String {
        match kind {
            "import" | "system_import" => clean_c_include_name(raw_name),
            _ => raw_name.to_string(),
        }
    }

    fn map_symbol_type(&self, kind: &str) -> SymbolType {
        match kind {
            "class" => SymbolType::Class,
            "interface" => SymbolType::Interface,
            "method" => SymbolType::Method,
            _ => SymbolType::Function,
        }
    }

    fn map_relation_type(&self, kind: &str) -> CodeRelationType {
        match kind {
            "call" | "method_call" => CodeRelationType::Calls,
            "import" | "system_import" => CodeRelationType::Imports,
            "implements" => CodeRelationType::Implements,
            "extends" => CodeRelationType::Extends,
            _ => CodeRelationType::Calls,
        }
    }
}

pub fn get_language_support(lang: Language) -> Option<Box<dyn LanguageSupport>> {
    match lang {
        Language::Rust => Some(Box::new(RustSupport)),
        Language::Python => Some(Box::new(PythonSupport)),
        Language::TypeScript => Some(Box::new(TypeScriptSupport)),
        Language::JavaScript => Some(Box::new(JavaScriptSupport)),
        Language::Go => Some(Box::new(GoSupport)),
        Language::Java => Some(Box::new(JavaSupport)),
        Language::Dart => Some(Box::new(DartSupport)),
        Language::C => Some(Box::new(CSupport)),
        Language::Cpp => Some(Box::new(CppSupport)),
        Language::Swift => Some(Box::new(SwiftSupport)),
        Language::Kotlin => Some(Box::new(KotlinSupport)),
        Language::ObjectiveC => Some(Box::new(ObjectiveCSupport)),
        _ => None,
    }
}

#[cfg(test)]
mod grammar_compatibility_tests {
    use tree_sitter::Parser;

    fn accepts_language(language: tree_sitter::Language, source: &str) {
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn mobile_grammar_crates_match_tree_sitter_026_language_api() {
        accepts_language(tree_sitter_c::LANGUAGE.into(), "int main(void) { return 0; }");
        accepts_language(tree_sitter_cpp::LANGUAGE.into(), "int main() { return 0; }");
        accepts_language(tree_sitter_swift::LANGUAGE.into(), "func main() { print(\"ok\") }");
        accepts_language(tree_sitter_kotlin_ng::LANGUAGE.into(), "fun main() { println(\"ok\") }");
        accepts_language(
            tree_sitter_objc::LANGUAGE.into(),
            "@interface Foo @end @implementation Foo @end",
        );
    }
}
