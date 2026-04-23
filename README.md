# rlox

A Rust implementation of the Lox bytecode VM, following Part III of Bob Nystrom's [*Crafting Interpreters*](https://craftinginterpreters.com).

Lox is a dynamically typed scripting language. clox is its bytecode-compiled implementation written in C. This is that, but in Rust so I called it rlox. Very original, no?

## Example

```lox
// basic capture
fun make_adder(x) {
  fun add(y) {
    return x + y;
  }
  return add;
}

var add5 = make_adder(5);
print add5(3);   // 8
print add5(10);  // 15

// mutation over multiple calls
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

// upvalue closed over after block scope exits
fun make_adder2(x) {
  {
    var offset = 100;
    fun add(y) {
      return x + y + offset;
    }
    return add;
  }
}

var add5_offset = make_adder2(5);
print add5_offset(3);  // 108
```

```
8
15
1
2
3
108
```

## Building

```
cargo build --release
```

## Usage

```
# Run a file
rlox <file>

# Run the included example
rlox test.lox

# REPL
rlox

# Flags
-d, --debug      Print bytecode disassembly before execution
-t, --tracing    Trace stack and instruction at runtime
-g, --gc-log     Print GC activity (allocations, mark/sweep, collections)

# Run the example with disassembly
rlox --debug test.lox
```

## Implementation status

**Implemented**

- Single-pass bytecode compiler with Pratt parser
- Stack-based VM with call frames
- Types: `f64` numbers, booleans, nil, strings
- Arithmetic: `+` `-` `*` `/` and unary `-`
- Comparison: `<` `>` `<=` `>=` `==` `!=`
- String concatenation and interning
- Global and local variables with block scoping
- `if` / `else`
- `while` and `for` loops
- Logical `and` / `or` (short-circuit evaluation)
- `print` statement
- Functions with parameters and return values
- Recursion
- Stack traces on runtime errors
- Panic-mode error recovery with synchronization
- Native functions (`clock`, `sqrt`) with `fn(&[Value]) -> Result<Value>` signature
- REPL continues after compile/runtime errors
- Closures with full upvalue capture
  - Open upvalues: `location` points into the stack while the enclosing frame is live
  - Closed upvalues: self-referential heap objects — `location` redirects to `closed` on frame exit
  - `CloseUpvalue` opcode for mid-function block scope exits
  - Shared upvalues: multiple closures over the same variable reuse a single upvalue object
  - Upvalue relay: closures more than one scope level deep chain descriptors through intermediate contexts
- Mark-and-sweep garbage collector
  - Tri-color marking: white (unvisited), grey (marked, children untraced), black (fully traced)
  - Roots: value stack, call frames, open upvalues, globals
  - Transitive tracing via grey worklist — compiler allocations reached through constants pool chain
  - Weak string intern table: dead strings evicted before sweep via `table_remove_white`
  - Heap size tracked per-object including owned allocations (`String`, `Vec`, `Chunk` buffers)
  - GC threshold grows by 2× after each collection

**Not yet started**

- Classes and inheritance
