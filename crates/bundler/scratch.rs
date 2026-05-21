use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn main() {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, "export const x = 1;", SourceType::mjs()).parse();
    for stmt in &ret.program.body {
        println!("{:?}", stmt);
    }
}
