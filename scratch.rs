use oxc_allocator::Allocator;
use oxc_ast::ast::{CallExpression, Argument, Expression, StringLiteral};
fn foo<'a>(arg: &'a Argument<'a>) {
    if let Argument::StringLiteral(s) = arg {
        println!("{}", s.value);
    }
}
fn main() {}
