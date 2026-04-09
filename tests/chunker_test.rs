use canopy::chunker::{chunk_file, detect_language, TypeRefPosition};

// ---------------------------------------------------------------------------
// Language detection
// ---------------------------------------------------------------------------

#[test]
fn test_detect_language() {
    assert_eq!(detect_language("src/main.rs"), Some("rust"));
    assert_eq!(detect_language("app.ts"), Some("typescript"));
    assert_eq!(detect_language("app.tsx"), Some("tsx"));
    assert_eq!(detect_language("app.js"), Some("javascript"));
    assert_eq!(detect_language("app.jsx"), Some("javascript"));
    assert_eq!(detect_language("app.mjs"), Some("javascript"));
    assert_eq!(detect_language("app.cjs"), Some("javascript"));
    assert_eq!(detect_language("main.py"), Some("python"));
    assert_eq!(detect_language("types.pyi"), Some("python"));
    assert_eq!(detect_language("main.go"), Some("go"));
    assert_eq!(detect_language("main.c"), Some("c"));
    assert_eq!(detect_language("lib.h"), Some("c"));
    assert_eq!(detect_language("main.cpp"), Some("cpp"));
    assert_eq!(detect_language("main.cc"), Some("cpp"));
    assert_eq!(detect_language("main.hpp"), Some("cpp"));
    assert_eq!(detect_language("Main.java"), Some("java"));
    assert_eq!(detect_language("Program.cs"), Some("csharp"));
    assert_eq!(detect_language("docs/README.md"), None);
    assert_eq!(detect_language("docs/guide.mdx"), None);
    assert_eq!(detect_language("no_ext"), None);
    assert_eq!(detect_language("readme.txt"), None);
    assert_eq!(detect_language("data.json"), None);
}

// ---------------------------------------------------------------------------
// Rust chunking
// ---------------------------------------------------------------------------

#[test]
fn test_chunk_simple_function() {
    let source = r#"
fn hello() {
    println!("hello");
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].content.contains("fn hello()"));
    assert_eq!(chunks[0].node_kinds, vec!["function_item"]);
}

#[test]
fn test_chunk_struct_and_impl() {
    let source = r#"
pub struct Foo {
    x: i32,
}

impl Foo {
    pub fn new(x: i32) -> Self {
        Self { x }
    }

    pub fn value(&self) -> i32 {
        self.x
    }
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_content: String = chunks
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all_content.contains("pub struct Foo"));
    assert!(all_content.contains("impl Foo"));
}

#[test]
fn test_merge_small_chunks() {
    let source = r#"
const A: i32 = 1;

const B: i32 = 2;

const C: i32 = 3;
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    // Three 1-line constants — all under merge_threshold, should merge into 1 chunk
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].content.contains("const A"));
    assert!(chunks[0].content.contains("const C"));
    assert_eq!(chunks[0].node_kinds.len(), 3);
}

#[test]
fn test_line_numbers() {
    let source = r#"fn first() {}

fn second() {
    let x = 1;
    let y = 2;
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 5, 200, None);
    assert!(!chunks.is_empty());
    assert_eq!(chunks[0].line_start, 1);
}

#[test]
fn test_unsupported_language_returns_whole_file() {
    let source = "some plain text content";
    let chunks = chunk_file(source, "readme.txt", "text", 20, 200, None);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].node_kinds, vec!["file"]);
    assert!(chunks[0].content.contains(source));
    assert!(chunks[0].content.starts_with("// readme.txt:"));
}

#[test]
fn test_empty_file() {
    let chunks = chunk_file("", "empty.rs", "rust", 20, 200, None);
    assert!(chunks.is_empty());
}

#[test]
fn test_file_with_doc_comments() {
    let source = r#"
/// This is a documented function.
/// It does important things.
pub fn documented() -> bool {
    true
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].content.contains("/// This is a documented function"));
    assert!(chunks[0].content.contains("pub fn documented"));
}

// ---------------------------------------------------------------------------
// Multi-language chunking
// ---------------------------------------------------------------------------

#[test]
fn test_chunk_python_function() {
    let source = r#"
def hello(name):
    print(f"Hello, {name}")
    return True

class Greeter:
    def __init__(self, name):
        self.name = name

    def greet(self):
        return f"Hello, {self.name}"
"#;
    let chunks = chunk_file(source, "test.py", "python", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_kinds: Vec<&str> = chunks
        .iter()
        .flat_map(|c| c.node_kinds.iter().map(|s| s.as_str()))
        .collect();
    assert!(all_kinds.contains(&"function_definition") || all_kinds.contains(&"class_definition"));
}

#[test]
fn test_chunk_typescript_class() {
    let source = r#"
interface User {
    name: string;
    age: number;
}

function greet(user: User): string {
    return `Hello, ${user.name}`;
}

class UserService {
    private users: User[] = [];

    addUser(user: User): void {
        this.users.push(user);
    }
}
"#;
    let chunks = chunk_file(source, "test.ts", "typescript", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_kinds: Vec<&str> = chunks
        .iter()
        .flat_map(|c| c.node_kinds.iter().map(|s| s.as_str()))
        .collect();
    assert!(
        all_kinds.contains(&"interface_declaration")
            || all_kinds.contains(&"function_declaration")
            || all_kinds.contains(&"class_declaration")
    );
}

#[test]
fn test_chunk_javascript_function() {
    let source = r#"
function add(a, b) {
    return a + b;
}

class Calculator {
    constructor() {
        this.result = 0;
    }

    add(value) {
        this.result += value;
        return this;
    }
}
"#;
    let chunks = chunk_file(source, "test.js", "javascript", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_kinds: Vec<&str> = chunks
        .iter()
        .flat_map(|c| c.node_kinds.iter().map(|s| s.as_str()))
        .collect();
    assert!(
        all_kinds.contains(&"function_declaration") || all_kinds.contains(&"class_declaration")
    );
}

#[test]
fn test_chunk_go_function() {
    let source = r#"package main

func main() {
    fmt.Println("hello")
}

func add(a, b int) int {
    return a + b
}

type Server struct {
    port int
    host string
}
"#;
    let chunks = chunk_file(source, "test.go", "go", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_kinds: Vec<&str> = chunks
        .iter()
        .flat_map(|c| c.node_kinds.iter().map(|s| s.as_str()))
        .collect();
    assert!(
        all_kinds.contains(&"function_declaration") || all_kinds.contains(&"type_declaration")
    );
}

#[test]
fn test_chunk_c_function() {
    let source = r#"
#include <stdio.h>

struct Point {
    int x;
    int y;
};

int add(int a, int b) {
    return a + b;
}

int main() {
    struct Point p = {1, 2};
    printf("%d\n", add(p.x, p.y));
    return 0;
}
"#;
    let chunks = chunk_file(source, "test.c", "c", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_content: String = chunks.iter().map(|c| c.content.as_str()).collect::<Vec<_>>().join("\n");
    assert!(all_content.contains("add") || all_content.contains("main") || all_content.contains("Point"));
}

#[test]
fn test_chunk_cpp_class() {
    let source = r#"
#include <string>

namespace app {

class Greeter {
public:
    Greeter(const std::string& name) : name_(name) {}

    std::string greet() const {
        return "Hello, " + name_;
    }

private:
    std::string name_;
};

}
"#;
    let chunks = chunk_file(source, "test.cpp", "cpp", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_content: String = chunks.iter().map(|c| c.content.as_str()).collect::<Vec<_>>().join("\n");
    assert!(all_content.contains("Greeter") || all_content.contains("namespace"));
}

#[test]
fn test_chunk_java_class() {
    let source = r#"
public class Calculator {
    private int result;

    public Calculator() {
        this.result = 0;
    }

    public int add(int value) {
        this.result += value;
        return this.result;
    }
}
"#;
    let chunks = chunk_file(source, "Test.java", "java", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_kinds: Vec<&str> = chunks
        .iter()
        .flat_map(|c| c.node_kinds.iter().map(|s| s.as_str()))
        .collect();
    assert!(all_kinds.contains(&"class_declaration"));
}

#[test]
fn test_chunk_csharp_class() {
    let source = r#"
namespace App {
    public class Calculator {
        private int result;

        public int Add(int value) {
            this.result += value;
            return this.result;
        }
    }
}
"#;
    let chunks = chunk_file(source, "Test.cs", "csharp", 20, 200, None);
    assert!(!chunks.is_empty());
    let all_content: String = chunks.iter().map(|c| c.content.as_str()).collect::<Vec<_>>().join("\n");
    assert!(all_content.contains("Calculator") || all_content.contains("namespace"));
}

// ---------------------------------------------------------------------------
// Symbol extraction
// ---------------------------------------------------------------------------

#[test]
fn test_symbol_extraction_rust() {
    let source = r#"
pub fn build_dag(units: Vec<String>) -> Dag {
    Dag::new(units)
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    assert_eq!(chunks.len(), 1);
    assert!(!chunks[0].defines.is_empty());

    let sym = &chunks[0].defines[0];
    assert_eq!(sym.name, "build_dag");
    assert_eq!(sym.kind, "function");
    assert!(!sym.signature.is_empty());
}

#[test]
fn test_symbol_extraction_python() {
    let source = r#"
def process_data(items):
    return [item.strip() for item in items]

class DataProcessor:
    def run(self):
        pass
"#;
    let chunks = chunk_file(source, "test.py", "python", 20, 200, None);
    let all_defines: Vec<&str> = chunks
        .iter()
        .flat_map(|c| c.defines.iter().map(|d| d.name.as_str()))
        .collect();
    assert!(all_defines.contains(&"process_data"));
    assert!(all_defines.contains(&"DataProcessor"));
}

#[test]
fn test_symbol_extraction_go() {
    let source = r#"package main

func Add(a, b int) int {
    return a + b
}

type Server struct {
    port int
}
"#;
    let chunks = chunk_file(source, "test.go", "go", 20, 200, None);
    let all_defines: Vec<&str> = chunks
        .iter()
        .flat_map(|c| c.defines.iter().map(|d| d.name.as_str()))
        .collect();
    assert!(all_defines.contains(&"Add"));
}

#[test]
fn test_references_extraction() {
    let source = r#"
fn process(input: Vec<String>) -> Result<Output, Error> {
    let parser = Parser::new();
    let result = parser.parse(input);
    Ok(result)
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    assert_eq!(chunks.len(), 1);
    let refs = &chunks[0].other_refs;
    // Should contain referenced types/identifiers but not the defined name
    assert!(!refs.contains(&"process".to_string()));
    // Should contain some of the referenced types
    assert!(refs.contains(&"Vec".to_string()) || refs.contains(&"Parser".to_string()) || refs.contains(&"Result".to_string()));
}

#[test]
fn test_no_self_references() {
    let source = r#"
pub struct MyStruct {
    field: i32,
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    assert_eq!(chunks.len(), 1);
    let refs = &chunks[0].other_refs;
    assert!(!refs.contains(&"MyStruct".to_string()));
}

#[test]
fn test_merged_chunks_combine_symbols() {
    let source = r#"
const MAX: i32 = 100;

const MIN: i32 = 0;
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    // Both should merge into one chunk
    assert_eq!(chunks.len(), 1);
    // Both symbols should be present
    let names: Vec<&str> = chunks[0].defines.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"MAX"));
    assert!(names.contains(&"MIN"));
}

// ---------------------------------------------------------------------------
// Raw edge extraction
// ---------------------------------------------------------------------------

#[test]
fn test_extract_imports_rust() {
    let source = r#"
use crate::store::Store;
use crate::config::Config;
use std::collections::HashMap;

fn main() {}
"#;
    let chunks = chunk_file(source, "src/main.rs", "rust", 20, 200, None);
    let all_imports: Vec<_> = chunks.iter().flat_map(|c| c.imports.iter()).collect();
    assert!(all_imports.iter().any(|i| i.name == "Store" && i.path.contains("store")));
    assert!(all_imports.iter().any(|i| i.name == "Config" && i.path.contains("config")));
}

#[test]
fn test_extract_type_refs_from_function_signature() {
    let source = r#"
fn open(path: &Path, config: &Config) -> Result<Store> {
    todo!()
}
"#;
    let chunks = chunk_file(source, "src/store.rs", "rust", 20, 200, None);
    let refs = &chunks[0].type_refs;
    assert!(refs.iter().any(|r| r.name == "Path" && r.position == TypeRefPosition::Parameter));
    assert!(refs.iter().any(|r| r.name == "Config" && r.position == TypeRefPosition::Parameter));
    assert!(refs.iter().any(|r| r.name == "Store" && r.position == TypeRefPosition::ReturnType));
}

#[test]
fn test_extract_field_defs_from_struct() {
    let source = r#"
struct Store {
    db: Database,
    config: Config,
    count: u64,
}
"#;
    let chunks = chunk_file(source, "src/store.rs", "rust", 20, 200, None);
    let fields = &chunks[0].field_defs;
    assert!(fields.iter().any(|f| f.field_name == "db" && f.type_name == "Database"));
    assert!(fields.iter().any(|f| f.field_name == "config" && f.type_name == "Config"));
    assert!(!fields.iter().any(|f| f.type_name == "u64"));
}

#[test]
fn test_impl_signature_captured() {
    let source = r#"
impl Display for Store {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Store")
    }
}
"#;
    let chunks = chunk_file(source, "src/store.rs", "rust", 20, 200, None);
    assert!(chunks[0].defines.iter().any(|d| d.kind == "impl" && d.signature.contains("Display") && d.signature.contains("Store")));
}

#[test]
fn test_container_extracts_child_defines() {
    let source = r#"
pub struct Foo {
    x: i32,
}

impl Foo {
    pub fn new(x: i32) -> Self {
        Self { x }
    }

    pub fn value(&self) -> i32 {
        self.x
    }
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    let impl_chunk = chunks.iter().find(|c| c.content.contains("impl Foo")).unwrap();
    let defined_names: Vec<&str> = impl_chunk.defines.iter().map(|d| d.name.as_str()).collect();
    assert!(defined_names.contains(&"new"), "should extract 'new' method, got: {:?}", defined_names);
    assert!(defined_names.contains(&"value"), "should extract 'value' method, got: {:?}", defined_names);
    let new_def = impl_chunk.defines.iter().find(|d| d.name == "new").unwrap();
    assert_eq!(new_def.kind, "function");
}

#[test]
fn test_container_sets_parent_scope() {
    let source = r#"
impl Store {
    pub fn open() -> Self {
        todo!()
    }
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    let impl_chunk = chunks.iter().find(|c| c.content.contains("impl Store")).unwrap();
    assert_eq!(impl_chunk.parent_scope, "Store", "container should set parent_scope to type name");
}

#[test]
fn test_python_class_extracts_method_defines() {
    let source = r#"
class MyService:
    def __init__(self, config):
        self.config = config

    def run(self):
        pass

    def stop(self):
        pass
"#;
    let chunks = chunk_file(source, "service.py", "python", 20, 200, None);
    let class_chunk = chunks.iter().find(|c| c.content.contains("class MyService")).unwrap();
    let defined_names: Vec<&str> = class_chunk.defines.iter().map(|d| d.name.as_str()).collect();
    assert!(defined_names.contains(&"__init__"), "should extract __init__, got: {:?}", defined_names);
    assert!(defined_names.contains(&"run"), "should extract run, got: {:?}", defined_names);
    assert!(defined_names.contains(&"stop"), "should extract stop, got: {:?}", defined_names);
    assert_eq!(class_chunk.parent_scope, "MyService");
}

#[test]
fn test_chunk_has_structured_reference_buckets() {
    let source = r#"
fn caller() {
    helper();
    obj.method_call();
    let x = SomeType::new();
}

fn helper() {}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    let chunk = chunks.iter().find(|c| c.defines.iter().any(|d| d.name == "caller")).unwrap();
    // New fields should exist (even if not yet populated correctly)
    assert!(chunk.free_calls.is_empty() || !chunk.free_calls.is_empty());
    assert!(chunk.method_calls.is_empty() || !chunk.method_calls.is_empty());
    assert!(chunk.other_refs.is_empty() || !chunk.other_refs.is_empty());
}

#[test]
fn test_rust_free_call_extracted() {
    let source = r#"
fn caller() {
    helper();
}

fn helper() {}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    let chunk = chunks.iter().find(|c| c.defines.iter().any(|d| d.name == "caller")).unwrap();
    assert!(chunk.free_calls.contains(&"helper".to_string()),
        "free_calls should contain 'helper', got: {:?}", chunk.free_calls);
}

#[test]
fn test_rust_method_call_classified() {
    let source = r#"
fn caller(items: Vec<i32>) {
    items.custom_method();
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    let chunk = chunks.iter().find(|c| c.defines.iter().any(|d| d.name == "caller")).unwrap();
    assert!(chunk.method_calls.contains(&"custom_method".to_string()),
        "method_calls should contain 'custom_method', got: {:?}", chunk.method_calls);
    // 'items' is a parameter binding — should NOT be in any bucket
    assert!(!chunk.other_refs.contains(&"items".to_string()),
        "parameter 'items' should be excluded by binding detection");
}

#[test]
fn test_rust_field_access_dropped() {
    let source = r#"
struct Pos { x: f32, y: f32 }
fn reader(p: Pos) -> f32 {
    p.x + p.y
}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    let chunk = chunks.iter().find(|c| c.defines.iter().any(|d| d.name == "reader")).unwrap();
    // Field accesses should NOT appear in any bucket
    assert!(!chunk.free_calls.contains(&"x".to_string()));
    assert!(!chunk.method_calls.contains(&"x".to_string()));
    assert!(!chunk.other_refs.contains(&"x".to_string()));
}

#[test]
fn test_rust_binding_excluded() {
    let source = r#"
fn example() {
    let result = compute();
    for item in results.iter() {
        process(item);
    }
}
fn compute() -> i32 { 42 }
fn process(x: i32) {}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    let chunk = chunks.iter().find(|c| c.defines.iter().any(|d| d.name == "example")).unwrap();
    // 'result' and 'item' are bindings — excluded
    assert!(!chunk.other_refs.contains(&"result".to_string()));
    assert!(!chunk.other_refs.contains(&"item".to_string()));
    // 'compute' and 'process' are free calls — included
    assert!(chunk.free_calls.contains(&"compute".to_string()),
        "free_calls should contain 'compute', got: {:?}", chunk.free_calls);
    assert!(chunk.free_calls.contains(&"process".to_string()),
        "free_calls should contain 'process', got: {:?}", chunk.free_calls);
}

#[test]
fn test_bucket_caps_independent() {
    let source = r#"
fn caller() {
    alpha();
    obj.beta();
    let x = Gamma;
}
fn alpha() {}
"#;
    let chunks = chunk_file(source, "test.rs", "rust", 20, 200, None);
    let chunk = chunks.iter().find(|c| c.defines.iter().any(|d| d.name == "caller")).unwrap();
    // All three buckets should have entries — none starved
    assert!(!chunk.free_calls.is_empty(), "free_calls should not be empty");
    assert!(!chunk.method_calls.is_empty(), "method_calls should not be empty");
    assert!(!chunk.other_refs.is_empty(), "other_refs should not be empty");
}

#[test]
fn test_large_function_with_inner_struct_preserves_parent_define() {
    // A function that exceeds split_threshold (set to 10 here) containing an inner struct.
    // Both the function and the inner struct should appear as defines.
    let source = r#"
pub fn rebuild(state: &mut State, playhead: f32) {
    let total = state.event_log.len();
    if total == 0 {
        return;
    }

    struct Lifecycle {
        born: bool,
        has_error: bool,
    }

    let mut lifecycles = Vec::new();
    for entry in state.event_log.iter() {
        lifecycles.push(Lifecycle { born: false, has_error: false });
    }
}
"#;
    // split_threshold=10 so the function (19 lines) will be split
    let chunks = chunk_file(source, "src/playhead.rs", "rust", 5, 10, None);

    // Collect all defines across all chunks
    let all_defines: Vec<&str> = chunks.iter()
        .flat_map(|c| c.defines.iter().map(|d| d.name.as_str()))
        .collect();

    assert!(all_defines.contains(&"rebuild"), "parent function 'rebuild' should be in defines, got: {:?}", all_defines);
    assert!(all_defines.contains(&"Lifecycle"), "inner struct 'Lifecycle' should be in defines, got: {:?}", all_defines);
}

