pub mod extractor;
pub mod languages;

use std::path::Path;

use crate::codebase::scanner::detect_language_with_content;
use crate::types::Language;
use crate::types::symbol::{CodeReference, CodeSymbol};

use extractor::Extractor;

pub struct CodeParser;

impl CodeParser {
    pub(crate) fn detect_language_for_parse(path: &Path, content: &str) -> Language {
        detect_language_with_content(path, content)
    }

    pub fn parse_file(
        path: &Path,
        content: &str,
        project_id: &str,
    ) -> (Vec<CodeSymbol>, Vec<CodeReference>) {
        let language = Self::detect_language_for_parse(path, content);
        let Some(mut extractor) = Extractor::new(language) else {
            return (vec![], vec![]);
        };

        extractor.parse(content, path.to_string_lossy().as_ref(), project_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::symbol::{CodeRelationType, SymbolType};
    use std::path::PathBuf;

    #[test]
    fn test_parser_crash() {
        let content = "fn test() {}";
        let path = PathBuf::from("test.rs");
        let (symbols, _) = CodeParser::parse_file(&path, content, "test");
        assert!(!symbols.is_empty());
    }

    #[test]
    fn parser_flow_uses_content_aware_header_detection() {
        let objc_header = "@interface ViewController : NSObject\n@end";
        let cpp_header = "namespace demo { template <typename T> class Box {}; }";
        let c_header = "typedef struct { int count; } Counter;";

        assert_eq!(
            CodeParser::detect_language_for_parse(Path::new("ViewController.h"), objc_header),
            Language::ObjectiveC
        );
        assert_eq!(
            CodeParser::detect_language_for_parse(Path::new("box.h"), cpp_header),
            Language::Cpp
        );
        assert_eq!(
            CodeParser::detect_language_for_parse(Path::new("counter.h"), c_header),
            Language::C
        );
    }

    #[test]
    fn test_rust_call_extraction() {
        let content = r#"
fn main() {
    let x = foo();
    bar(x);
}

fn foo() -> i32 { 42 }
fn bar(x: i32) {}
"#;
        let path = PathBuf::from("test.rs");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== SYMBOLS ===");
        for s in &symbols {
            println!(
                "  {} ({:?}) at line {}",
                s.name, s.symbol_type, s.start_line
            );
        }

        println!("\n=== REFERENCES ===");
        for r in &refs {
            println!(
                "  {} -> {} ({:?}) at line {}",
                r.from_symbol, r.to_symbol, r.relation_type, r.line
            );
        }

        // Should have 3 functions: main, foo, bar
        assert_eq!(symbols.len(), 3, "Expected 3 symbols");

        // Should have calls: main->foo, main->bar
        let calls: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.relation_type, CodeRelationType::Calls))
            .collect();

        println!("\n=== CALLS ONLY ===");
        for c in &calls {
            println!("  {} -> {}", c.from_symbol, c.to_symbol);
        }

        assert!(
            calls.len() >= 2,
            "Expected at least 2 calls, got {}",
            calls.len()
        );
    }

    #[test]
    fn test_dart_symbol_extraction() {
        let content = r#"
import 'package:flutter/material.dart';

class MyWidget extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Container();
  }

  void _handleTap() {}
}

void main() {
  runApp(MyApp());
}

enum AppState {
  loading,
  ready,
  error,
}

mixin LoggingMixin {
  void log(String message) {
    print(message);
  }
}

extension StringExt on String {
  String capitalize() {
    return '${this[0].toUpperCase()}${substring(1)}';
  }
}
"#;
        let path = PathBuf::from("test.dart");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== DART SYMBOLS ===");
        for s in &symbols {
            println!(
                "  {} ({:?}) at lines {}-{}",
                s.name, s.symbol_type, s.start_line, s.end_line
            );
        }

        println!("\n=== DART REFERENCES ===");
        for r in &refs {
            println!(
                "  {} -> {} ({:?}) at line {}",
                r.from_symbol, r.to_symbol, r.relation_type, r.line
            );
        }

        assert!(
            symbols.len() >= 5,
            "Expected at least 5 symbols, got {}. Names: {:?}",
            symbols.len(),
            symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        assert!(
            symbols.iter().any(|s| s.name == "MyWidget"),
            "Should find class MyWidget"
        );
        assert!(
            symbols.iter().any(|s| s.name == "main"),
            "Should find function main"
        );
        assert!(
            symbols.iter().any(|s| s.name == "AppState"),
            "Should find enum AppState"
        );
    }

    #[test]
    fn test_dart_ast_dump() {
        use tree_sitter::Parser;

        let mut parser = Parser::new();
        let lang: tree_sitter::Language = tree_sitter_dart_orchard::LANGUAGE.into();
        parser.set_language(&lang).unwrap();

        let code = r#"
import 'package:flutter/material.dart';

class MyService {
  final ApiClient client;

  void doWork() {
    print("hello");
    someFunction(42);
    client.fetchData(url);
    widget?.build(context);
    Navigator.of(context).push(route);
    list..add(1)..add(2);
    setState(() {});
    Future.delayed(Duration(seconds: 1));
  }
}

void topLevelFunction() {
  final result = compute(42);
}
"#;

        let tree = parser.parse(code, None).unwrap();
        dump_node(tree.root_node(), code, 0);
    }

    #[test]
    fn test_dart_reference_extraction() {
        let content = r#"
import 'package:flutter/material.dart';

class MyWidget extends StatelessWidget {
  final ApiClient client;

  Widget build(BuildContext context) {
    print("hello");
    client.fetchData("url");
    widget?.rebuild(context);
    setState(() {});
    list..add(1)..add(2);
    return Container();
  }
}

void main() {
  runApp(MyApp());
}
"#;
        let path = PathBuf::from("test.dart");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== DART SYMBOLS ===");
        for s in &symbols {
            println!(
                "  {} ({:?}) L{}-{}",
                s.name, s.symbol_type, s.start_line, s.end_line
            );
        }

        println!("\n=== DART REFERENCES ===");
        for r in &refs {
            println!(
                "  {} -> {} ({:?}) L{}",
                r.from_symbol, r.to_symbol, r.relation_type, r.line
            );
        }

        let calls: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.relation_type, CodeRelationType::Calls))
            .collect();

        let imports: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.relation_type, CodeRelationType::Imports))
            .collect();

        println!("\n=== CALLS ({}) ===", calls.len());
        for c in &calls {
            println!("  {} -> {}", c.from_symbol, c.to_symbol);
        }
        println!("\n=== IMPORTS ({}) ===", imports.len());
        for i in &imports {
            println!("  {}", i.to_symbol);
        }

        // Import works
        assert!(!imports.is_empty(), "Should find at least 1 import");

        // Function calls found
        assert!(
            calls.len() >= 2,
            "Should find at least 2 calls, got {}. All refs: {:?}",
            calls.len(),
            refs.iter()
                .map(|r| (&r.from_symbol, &r.to_symbol, &r.relation_type))
                .collect::<Vec<_>>()
        );

        // Specific calls
        assert!(
            calls.iter().any(|c| c.to_symbol == "print"),
            "Should find call to 'print'"
        );
        assert!(
            calls.iter().any(|c| c.to_symbol == "runApp"),
            "Should find call to 'runApp'"
        );
    }

    #[test]
    fn test_dart_real_project_references() {
        // Test on real Dart file from mobile-odoo project
        let content = r#"
import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:sentry_flutter/sentry_flutter.dart';

class ErrorHandler {
  const ErrorHandler({GlobalErrorHandler? sentryHandler, GlobalErrorHandler? customLogger})
      : _sentryHandler = sentryHandler, _customLogger = customLogger;
  final GlobalErrorHandler? _sentryHandler;
  final GlobalErrorHandler? _customLogger;

  Future<void> handle(Object error, StackTrace stackTrace) async {
    await _customLogger?.call(error, stackTrace);
    await _sentryHandler?.call(error, stackTrace);
  }
}

Future<void> bootstrap({required Widget child}) async {
  FlutterError.onError = (FlutterErrorDetails details) {
    FlutterError.presentError(details);
    errorHandler.handle(details.exception, details.stack ?? StackTrace.empty);
  };

  await runZonedGuarded(
    () async {
      await _initializeWithMonitoring(dsn: dsn, child: child);
    },
    (error, stackTrace) async {
      await errorHandler.handle(error, stackTrace);
    },
  );
}

Future<void> _initializeWithMonitoring({required String dsn, required Widget child}) async {
  if (dsn.isNotEmpty) {
    await SentryFlutter.init((options) {
      options.dsn = dsn;
    }, appRunner: () => _initPostHogAndRun(child));
  }
}

void _initPostHogAndRun(Widget child) {
  final config = PostHogConfig(posthogKey);
  config.host = 'https://example.com';
  Posthog().setup(config);
  runApp(ProviderScope(overrides: [], child: child));
}
"#;
        let path = PathBuf::from("lib/app/bootstrap.dart");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== REAL PROJECT: SYMBOLS ({}) ===", symbols.len());
        for s in &symbols {
            println!(
                "  {} ({:?}) L{}-{}",
                s.name, s.symbol_type, s.start_line, s.end_line
            );
        }

        println!("\n=== REAL PROJECT: ALL REFERENCES ({}) ===", refs.len());
        for r in &refs {
            println!(
                "  {} -> {} ({:?}) L{}",
                r.from_symbol, r.to_symbol, r.relation_type, r.line
            );
        }

        let calls: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.relation_type, CodeRelationType::Calls))
            .collect();
        let imports: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.relation_type, CodeRelationType::Imports))
            .collect();

        println!("\n=== CALLS ({}) ===", calls.len());
        for c in &calls {
            println!("  {} -> {} (L{})", c.from_symbol, c.to_symbol, c.line);
        }
        println!("\n=== IMPORTS ({}) ===", imports.len());
        for i in &imports {
            println!("  {} (L{})", i.to_symbol, i.line);
        }

        // Imports
        assert!(
            imports.len() >= 3,
            "Should find at least 3 imports, got {}",
            imports.len()
        );

        // At least some calls should be found
        assert!(
            calls.len() >= 3,
            "Should find at least 3 calls, got {}",
            calls.len()
        );

        // Specific expected calls
        assert!(
            calls.iter().any(|c| c.to_symbol == "handle"),
            "Should find 'handle' method call"
        );
        assert!(
            calls.iter().any(|c| c.to_symbol == "runApp"),
            "Should find 'runApp' call"
        );
    }

    #[test]
    fn test_dart_extends_implements() {
        let content = r#"
import 'package:flutter/material.dart';

abstract class BaseRepository {
  void fetch();
}

mixin LoggingMixin {
  void log(String msg) {}
}

class UserRepository extends BaseRepository with LoggingMixin implements Serializable {
  @override
  void fetch() {
    log('fetching');
  }
}

class AdminRepository extends UserRepository implements Auditable, Cacheable {
  @override
  void fetch() {
    super.fetch();
  }
}
"#;
        let path = PathBuf::from("lib/models/repository.dart");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== EXTENDS/IMPLEMENTS: SYMBOLS ({}) ===", symbols.len());
        for s in &symbols {
            println!(
                "  {} ({:?}) L{}-{}",
                s.name, s.symbol_type, s.start_line, s.end_line
            );
        }

        println!("\n=== EXTENDS/IMPLEMENTS: REFERENCES ({}) ===", refs.len());
        for r in &refs {
            println!(
                "  {} -> {} ({:?}) L{}",
                r.from_symbol, r.to_symbol, r.relation_type, r.line
            );
        }

        let extends: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.relation_type, CodeRelationType::Extends))
            .collect();
        let implements: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.relation_type, CodeRelationType::Implements))
            .collect();

        println!("\n=== EXTENDS ({}) ===", extends.len());
        for e in &extends {
            println!("  {} extends {}", e.from_symbol, e.to_symbol);
        }
        println!("\n=== IMPLEMENTS ({}) ===", implements.len());
        for i in &implements {
            println!("  {} implements {}", i.from_symbol, i.to_symbol);
        }

        // UserRepository extends BaseRepository
        assert!(
            extends.iter().any(|e| e.to_symbol == "BaseRepository"),
            "Should find 'extends BaseRepository'"
        );

        // AdminRepository extends UserRepository
        assert!(
            extends.iter().any(|e| e.to_symbol == "UserRepository"),
            "Should find 'extends UserRepository'"
        );

        // UserRepository implements Serializable
        assert!(
            implements.iter().any(|i| i.to_symbol == "Serializable"),
            "Should find 'implements Serializable'"
        );

        // AdminRepository implements Auditable
        assert!(
            implements.iter().any(|i| i.to_symbol == "Auditable"),
            "Should find 'implements Auditable'"
        );

        // UserRepository with LoggingMixin (captured as implements)
        assert!(
            implements.iter().any(|i| i.to_symbol == "LoggingMixin"),
            "Should find 'with LoggingMixin' (as implements)"
        );
    }

    #[test]
    fn test_c_parser_contract_extracts_syntax_symbols_and_references() {
        let content = r#"
#include <stdio.h>
#include "worker.h"

typedef unsigned long WorkerId;

struct Worker {
    WorkerId id;
};

enum WorkerState {
    WorkerIdle,
    WorkerBusy,
};

static int helper(void) {
    return puts("help");
}

int run_worker(struct Worker *worker) {
    helper();
    printf("%lu", worker->id);
    return 0;
}
"#;
        let path = PathBuf::from("src/worker.c");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== C CONTRACT SYMBOLS ({}) ===", symbols.len());
        print_symbols(&symbols);
        println!("\n=== C CONTRACT REFERENCES ({}) ===", refs.len());
        print_refs(&refs);

        assert_symbol(&symbols, "WorkerId", SymbolType::Struct);
        assert_symbol(&symbols, "Worker", SymbolType::Struct);
        assert_symbol(&symbols, "WorkerState", SymbolType::Enum);
        assert_symbol(&symbols, "helper", SymbolType::Function);
        assert_symbol(&symbols, "run_worker", SymbolType::Function);
        assert_ref(&refs, CodeRelationType::Imports, "stdio.h");
        assert_ref(&refs, CodeRelationType::Imports, "worker.h");
        assert_ref(&refs, CodeRelationType::Calls, "helper");
        assert_ref(&refs, CodeRelationType::Calls, "printf");
    }

    #[test]
    fn test_cpp_parser_contract_extracts_syntax_symbols_and_references() {
        let content = r#"
#include <vector>
#include "engine.hpp"

namespace app {
enum class Mode { Idle, Running };

class Engine {
public:
    Engine();
    void start();
};

Engine::Engine() {}

void Engine::start() {
    tick();
}

void tick() {
    log_event();
}
}

int main() {
    app::Engine engine;
    engine.start();
    return 0;
}
"#;
        let path = PathBuf::from("src/engine.cpp");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== C++ CONTRACT SYMBOLS ({}) ===", symbols.len());
        print_symbols(&symbols);
        println!("\n=== C++ CONTRACT REFERENCES ({}) ===", refs.len());
        print_refs(&refs);

        assert_symbol(&symbols, "app", SymbolType::Module);
        assert_symbol(&symbols, "Mode", SymbolType::Enum);
        assert_symbol(&symbols, "Engine", SymbolType::Class);
        assert_symbol(&symbols, "Engine", SymbolType::Method);
        assert_symbol(&symbols, "start", SymbolType::Method);
        assert_symbol(&symbols, "tick", SymbolType::Function);
        assert_symbol(&symbols, "main", SymbolType::Function);
        assert_ref(&refs, CodeRelationType::Imports, "vector");
        assert_ref(&refs, CodeRelationType::Imports, "engine.hpp");
        assert_ref(&refs, CodeRelationType::Calls, "tick");
        assert_ref(&refs, CodeRelationType::Calls, "log_event");
        assert_ref(&refs, CodeRelationType::Calls, "start");
    }

    #[test]
    fn test_swift_parser_contract_extracts_syntax_symbols_and_references() {
        let content = r#"
import Foundation
import SwiftUI

protocol Renderable {
    func render()
}

struct Model {
    let id: String
}

enum ScreenState {
    case loading
    case ready
}

class ScreenController: Renderable {
    init(model: Model) {
        configure(model)
    }

    func render() {
        draw()
    }
}

extension ScreenController {
    func refresh() {
        render()
    }
}

func bootstrap() {
    ScreenController(model: Model(id: "1")).refresh()
}
"#;
        let path = PathBuf::from("Sources/App/ScreenController.swift");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== SWIFT CONTRACT SYMBOLS ({}) ===", symbols.len());
        print_symbols(&symbols);
        println!("\n=== SWIFT CONTRACT REFERENCES ({}) ===", refs.len());
        print_refs(&refs);

        assert_symbol(&symbols, "Renderable", SymbolType::Interface);
        assert_symbol(&symbols, "Model", SymbolType::Struct);
        assert_symbol(&symbols, "ScreenState", SymbolType::Enum);
        assert_symbol(&symbols, "ScreenController", SymbolType::Class);
        assert_symbol(&symbols, "init", SymbolType::Method);
        assert_symbol(&symbols, "render", SymbolType::Method);
        assert_symbol(&symbols, "refresh", SymbolType::Method);
        assert_symbol(&symbols, "bootstrap", SymbolType::Function);
        assert_ref(&refs, CodeRelationType::Imports, "Foundation");
        assert_ref(&refs, CodeRelationType::Imports, "SwiftUI");
        assert_ref(&refs, CodeRelationType::Calls, "configure");
        assert_ref(&refs, CodeRelationType::Calls, "draw");
        assert_ref(&refs, CodeRelationType::Calls, "render");
        assert_ref(&refs, CodeRelationType::Calls, "refresh");
    }

    #[test]
    fn test_kotlin_parser_contract_extracts_syntax_symbols_and_references() {
        let content = r#"
package com.example.app

import kotlinx.coroutines.delay
import kotlin.time.Duration

interface Repository {
    suspend fun load(): String
}

class UserRepository : Repository {
    companion object {
        fun create(): UserRepository = UserRepository()
    }

    override suspend fun load(): String {
        delay(1)
        return fetchUser()
    }
}

object UserCache {
    fun clear() {
        println("clear")
    }
}

suspend fun String.refreshWith(repository: Repository): String {
    return repository.load()
}

fun fetchUser(): String = "user"
"#;
        let path = PathBuf::from("src/main/kotlin/com/example/app/UserRepository.kt");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== KOTLIN CONTRACT SYMBOLS ({}) ===", symbols.len());
        print_symbols(&symbols);
        println!("\n=== KOTLIN CONTRACT REFERENCES ({}) ===", refs.len());
        print_refs(&refs);

        assert_symbol(&symbols, "com.example.app", SymbolType::Module);
        assert_symbol(&symbols, "Repository", SymbolType::Interface);
        assert_symbol(&symbols, "UserRepository", SymbolType::Class);
        assert_symbol(&symbols, "Companion", SymbolType::Class);
        assert_symbol(&symbols, "UserCache", SymbolType::Class);
        assert_symbol(&symbols, "create", SymbolType::Function);
        assert_symbol(&symbols, "load", SymbolType::Method);
        assert_symbol(&symbols, "refreshWith", SymbolType::Function);
        assert_symbol(&symbols, "fetchUser", SymbolType::Function);
        assert_ref(&refs, CodeRelationType::Imports, "kotlinx.coroutines.delay");
        assert_ref(&refs, CodeRelationType::Imports, "kotlin.time.Duration");
        assert_ref(&refs, CodeRelationType::Calls, "delay");
        assert_ref(&refs, CodeRelationType::Calls, "fetchUser");
        assert_ref(&refs, CodeRelationType::Calls, "load");
        assert_ref(&refs, CodeRelationType::Calls, "println");
    }

    #[test]
    fn test_objective_c_parser_contract_uses_gate_or_fallback_extraction() {
        let content = r#"
#import <Foundation/Foundation.h>
#import "Worker.h"

@protocol WorkerDelegate
- (void)workerDidFinish:(id)worker;
@end

@interface Worker : NSObject
- (instancetype)initWithName:(NSString *)name;
- (void)start;
@end

@implementation Worker
- (instancetype)initWithName:(NSString *)name {
    self = [super init];
    return self;
}

- (void)start {
    NSLog(@"start");
    [self notifyDelegate];
}

- (void)notifyDelegate {
}
@end
"#;
        let path = PathBuf::from("Sources/Worker.m");
        let (symbols, refs) = CodeParser::parse_file(&path, content, "test");

        println!("=== OBJECTIVE-C CONTRACT SYMBOLS ({}) ===", symbols.len());
        print_symbols(&symbols);
        println!("\n=== OBJECTIVE-C CONTRACT REFERENCES ({}) ===", refs.len());
        print_refs(&refs);

        assert_symbol(&symbols, "WorkerDelegate", SymbolType::Interface);
        assert_symbol(&symbols, "Worker", SymbolType::Class);
        assert_symbol(&symbols, "initWithName", SymbolType::Method);
        assert_symbol(&symbols, "start", SymbolType::Method);
        assert_symbol(&symbols, "notifyDelegate", SymbolType::Method);
        assert_ref(&refs, CodeRelationType::Imports, "Foundation/Foundation.h");
        assert_ref(&refs, CodeRelationType::Imports, "Worker.h");
        assert_ref(&refs, CodeRelationType::Calls, "init");
        assert_ref(&refs, CodeRelationType::Calls, "NSLog");
        assert_ref(&refs, CodeRelationType::Calls, "notifyDelegate");
    }

    fn assert_symbol(symbols: &[CodeSymbol], name: &str, symbol_type: SymbolType) {
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == name && symbol.symbol_type == symbol_type),
            "Should find {symbol_type:?} symbol '{name}'. Symbols: {:?}",
            symbols
                .iter()
                .map(|symbol| (&symbol.name, &symbol.symbol_type))
                .collect::<Vec<_>>()
        );
    }

    fn assert_ref(refs: &[CodeReference], relation_type: CodeRelationType, to_symbol: &str) {
        assert!(
            refs.iter().any(|reference| reference.relation_type == relation_type
                && reference.to_symbol == to_symbol),
            "Should find {relation_type:?} reference to '{to_symbol}'. References: {:?}",
            refs.iter()
                .map(|reference| (&reference.from_symbol, &reference.to_symbol, &reference.relation_type))
                .collect::<Vec<_>>()
        );
    }

    fn print_symbols(symbols: &[CodeSymbol]) {
        for symbol in symbols {
            println!(
                "  {} ({:?}) L{}-{}",
                symbol.name, symbol.symbol_type, symbol.start_line, symbol.end_line
            );
        }
    }

    fn print_refs(refs: &[CodeReference]) {
        for reference in refs {
            println!(
                "  {} -> {} ({:?}) L{}",
                reference.from_symbol,
                reference.to_symbol,
                reference.relation_type,
                reference.line
            );
        }
    }

    fn dump_node(node: tree_sitter::Node, source: &str, indent: usize) {
        if !node.is_named() {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dump_node(child, source, indent);
            }
            return;
        }
        let kind = node.kind();
        if kind == "comment" || kind == "documentation_comment" {
            return;
        }

        let text = node.utf8_text(source.as_bytes()).unwrap_or("???");
        let short = if text.len() > 60 {
            format!("{}...", &text[..60])
        } else {
            text.to_string()
        };
        let short = short.replace('\n', "\\n");

        println!(
            "{}{} [L{}] {:?}",
            "  ".repeat(indent),
            kind,
            node.start_position().row + 1,
            short
        );

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            dump_node(child, source, indent + 1);
        }
    }
}
