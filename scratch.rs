use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_codegen::CodeGenerator;

fn main() {
    let allocator = Allocator::default();
    let source_text = "const x: string = 'hello';";
    let source_type = SourceType::mjs().with_typescript(true);
    let mut ret = Parser::new(&allocator, source_text, source_type).parse();
    
    // Codegen without Transformer
    let printed = CodeGenerator::new().build(&ret.program).code;
    println!("WITHOUT TRANSFORM: {}", printed);
}
