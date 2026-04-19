# rlox

A Rust implementation of the Lox bytecode VM, following Part III of Bob Nystrom's [*Crafting Interpreters*](https://craftinginterpreters.com).

Lox is a dynamically typed scripting language. clox is its bytecode-compiled implementation. This is that, but in Rust, so it's called rlox.

## Example

```lox
var name = "world";

{
  var greeting = "Hello, " + name + "!";
  print greeting;
}

var x = 10;
var y = 3;

if (x > y) {
  print x - y;
} else {
  print y - x;
}
```

```
Hello, world!
7
```

## Building

```
cargo build --release
```

## Usage

```
# Run a file
rlox <file>

# REPL
rlox

# Flags
-d, --debug      Print bytecode disassembly before execution
-t, --tracing    Trace stack and instruction at runtime
```

## Implementation status

**Implemented**

- Single-pass bytecode compiler with Pratt parser
- Stack-based VM
- Types: `f64` numbers, booleans, nil, strings
- Arithmetic: `+` `-` `*` `/` and unary `-`
- Comparison: `<` `>` `<=` `>=` `==` `!=`
- String concatenation and interning
- Global and local variables with block scoping
- `if` / `else`
- `print` statement
- Panic-mode error recovery with synchronization

**In progress**

- `while` and `for` loops
- Logical `and` / `or` (short-circuit evaluation)
- Functions and closures
- Classes and inheritance
- Mark-and-sweep garbage collection (linked list infrastructure is in place)
