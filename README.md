# rlox

A Rust implementation of the Lox bytecode VM, following Part III of Bob Nystrom's [*Crafting Interpreters*](https://craftinginterpreters.com).

Lox is a dynamically typed scripting language with first-class functions, closures, and classes. This is a Rust port of clox, Nystrom's C implementation.

## Example

```lox
fun make_adder(x) {
  fun add(y) { return x + y; }
  return add;
}

var add5 = make_adder(5);
print add5(3);   // 8
print add5(10);  // 15

fun make_counter(start) {
  var count = start;
  fun increment() {
    count = count + 1;
    return count;
  }
  return increment;
}

var counter = make_counter(0);
print counter();  // 1
print counter();  // 2
print counter();  // 3

class Pair {}
var pair = Pair();
pair.first = 1;
pair.second = 2;
print pair.first + pair.second;  // 3
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
-d, --debug          Print bytecode disassembly before execution
-t, --tracing        Trace stack and instruction at runtime
-g, --gc-logging     Print GC activity (allocations, collections, frees)
```

## Implementation status

**Implemented**

- Single-pass bytecode compiler with Pratt parser
- Stack-based VM with call frames
- Types: `f64` numbers, booleans, nil, strings, functions, closures, classes, instances
- Arithmetic: `+` `-` `*` `/` and unary `-`
- Comparison: `<` `>` `<=` `>=` `==` `!=`
- Logical `and` / `or` with short-circuit evaluation
- String concatenation and interning
- Global and local variables with block scoping
- `if` / `else`, `while`, `for`
- `print` statement
- Functions with parameters, return values, and recursion
- Native functions (`clock`, `sqrt`)
- Closures with full upvalue capture
  - Open upvalues point into the stack while the enclosing frame is live
  - Closed upvalues are self-referential heap objects — `location` redirects to `closed` on frame exit
  - `CloseUpvalue` opcode handles mid-function block scope exits
  - Multiple closures over the same variable share a single upvalue object
  - Upvalue relay chains descriptors through intermediate scopes
- Classes and instances
  - `class` declarations with `OP_CLASS`
  - Instantiation via call syntax: `ClassName()`
  - Field get and set via dot notation (`instance.field`, `instance.field = value`)
- Mark-and-sweep garbage collector
  - Tri-color marking: grey worklist drives transitive tracing
  - Roots: value stack, call frames, open upvalues, globals
  - Weak string intern table evicts dead strings before sweep
  - Per-object heap size tracking including owned buffer allocations
  - Collection threshold doubles after each cycle
- REPL with error recovery (continues after compile and runtime errors)
- Panic-mode error synchronization in the compiler
- Stack traces on runtime errors

**Not yet implemented**

- Methods and `this`
- Initializers (`init`)
- Inheritance and `super`
