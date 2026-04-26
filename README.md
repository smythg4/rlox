# rlox

A Rust implementation of the Lox bytecode VM, following Part III of Bob Nystrom's [*Crafting Interpreters*](https://craftinginterpreters.com).

Lox is a dynamically typed scripting language with first-class functions, closures, and classes. This is a Rust port of clox, Nystrom's C implementation, complete through Chapter 29, with several post-book optimizations on top.

## Example

```lox
class Animal {
    init(name) { this.name = name; }
    speak() { print this.name + " makes a sound."; }
}

class Dog < Animal {
    speak() { print this.name + " barks."; }
}

var d = Dog("Rex");
d.speak();       // Rex barks.
d.describe();    // inherited from Animal if present

fun make_counter(start) {
    var n = start;
    fun increment() { n = n + 1; return n; }
    return increment;
}

var counter = make_counter(0);
print counter();  // 1
print counter();  // 2
```

## Building

```
cargo build --release
```

To build with NaN-boxing (encodes values as `u64`, ~10% faster):

```
cargo build --release --features nan-boxing
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

## Implementation

### Compiler

- Single-pass bytecode compiler with a Pratt parser
- Each function compiles into its own `ObjKind::Function` with an independent `Chunk`
- Compiler contexts are pushed/popped on a `Vec<CompilerContext>` stack; upvalue descriptors thread through intermediate scopes
- Panic-mode error synchronization — continues parsing after errors to report multiple diagnostics
- `FunctionKind` (`Script` / `Function` / `Method` / `Initializer`) controls slot-0 convention and implicit return behavior

### VM

- Stack-based VM with call frames; each frame carries a base pointer into the shared value stack
- `OpCode::Invoke` fuses `GetProperty` + `Call` for method calls on known receivers, skipping `BoundMethod` allocation
- `OpCode::SuperInvoke` fuses `GetSuper` + `Call` for `super.method()` calls
- `OpCode::Inherit` copies the superclass method table into the subclass at class-definition time (copy-down inheritance)
- `super` resolves via a synthetic `super` local captured as an upvalue, paired with `this` at slot 0

### Value Representation

`Value` has two representations, selected at compile time via a Cargo feature flag.

**Default — tagged enum** (`#[cfg(not(feature = "nan-boxing"))]`):

```rust
enum Value { Number(f64), Boolean(bool), Obj(*mut Obj), Nil }
```

**NaN-boxed** (`#[cfg(feature = "nan-boxing")]`):

```rust
struct Value(u64);
```

Numbers are stored as raw `f64` bits. Non-number types occupy the QNAN bit pattern (`0x7FFC_0000_0000_0000`) combined with a tag or a pointer:

| Tag | Constant |
|-----|----------|
| Nil | `QNAN \| 1` |
| False | `QNAN \| 2` |
| True | `QNAN \| 3` |
| Obj | `SIGN_BIT \| QNAN \| ptr` |

Both representations expose the same constructor and accessor API (`from_number`, `from_bool`, `from_obj`, `nil`, `is_number`, `as_obj`, etc.), so the VM and compiler are representation-agnostic.

### Types

| Type | Representation |
|------|---------------|
| Number | `f64` |
| Boolean | `bool` |
| Nil | unit variant / QNAN tag |
| String | interned `*mut Obj` |
| Function | `ObjKind::Function` (name, arity, chunk) |
| Closure | `ObjKind::Closure` (function pointer + upvalue list) |
| UpValue | `ObjKind::UpValue` (stack pointer → heap `closed` on exit) |
| Class | `ObjKind::Class` (name + method table) |
| Instance | `ObjKind::ClassInstance` (class pointer + field table) |
| BoundMethod | `ObjKind::BoundMethod` (receiver value + method pointer) |
| NativeFunction | `ObjKind::Native(fn(&[Value]) -> Result<Value>)` |

### Closures

- Open upvalues point directly into the stack while the enclosing frame is live
- `OpCode::CloseUpvalue` migrates the value to a heap `closed` field and redirects `location` to it
- Multiple closures over the same variable share a single `UpValue` object
- Upvalue relay chains propagate capture descriptors through intermediate scopes

### Garbage Collector

- Mark-and-sweep with a grey worklist for transitive tracing
- Roots: value stack, call frames, open upvalues, globals
- Weak string intern table — dead strings are evicted before the sweep phase
- Per-object heap size tracking (owned buffer capacity included)
- Collection threshold doubles after each cycle

### Language Features

- Arithmetic: `+` `-` `*` `/` and unary `-`
- Comparison: `<` `>` `<=` `>=` `==` `!=`
- Logical `and` / `or` with short-circuit evaluation
- String concatenation and interning
- Global and local variables with block scoping
- `if` / `else`, `while`, `for`
- `print` statement
- Functions with parameters, return values, and recursion
- Native functions: `clock`, `sqrt`
- Closures with full upvalue capture
- Classes, instances, methods, `this`
- `init` initializers — implicit `return this`, explicit `return` forbidden
- Bound methods as first-class values
- Inheritance (`<`) with method override
- `super` for superclass method dispatch, including `super.init()`
- REPL with error recovery (continues after compile and runtime errors)
- Stack traces on runtime errors

## Deviations from Nystrom's Implementation

### Structural

- Compiler contexts are a `Vec<CompilerContext>` stack rather than a linked list of `Compiler*` structs threaded via an `enclosing` pointer. Upvalue resolution walks by index instead of following a chain.
- `CallFrame::function` stores either a `Function` or `Closure` pointer. Nystrom always wraps functions in a closure before calling, so his frames only ever hold an `ObjClosure*`. A `resolve_function()` indirection handles both cases here.
- Open upvalues are a `Vec<*mut Obj>` with linear scan. Nystrom's is a linked list sorted by stack position, which makes `CloseUpvalue` O(1) — it only needs to inspect the head.
- Compiler roots are not marked directly. Nystrom has an explicit `markCompilerRoots` pass. Here, compiler-allocated objects are reachable transitively via `stack[0]` (the script closure), so they survive collection. This is an implicit assumption rather than an explicit invariant.

### Representation

- `Value` is a Rust enum or newtype struct depending on the `nan-boxing` feature flag. Nystrom uses a tagged union and discusses NaN-boxing as an optimization in Chapter 30.
- `ObjKind` is a Rust enum covering all heap-allocated types. Nystrom uses separate structs (`ObjString`, `ObjFunction`, `ObjClosure`, etc.) sharing a common `Obj` header via C struct embedding.
- Native functions are stored as `ObjKind::Native` on the heap so they participate in GC tracing. Nystrom similarly wraps them in `ObjNative`; they are not stored inline in `Value`.

### Hash Tables

- All identifier-keyed tables (globals, class method tables, instance field tables) use `VecMap<*mut Obj, Value>` — a hybrid map that does a linear scan over a `Vec` for tables up to a threshold size, then spills to `FnvHashMap`. Most Lox instances have only a handful of fields so hashing is never needed.
- Pointer keys rather than string-content keys throughout. Interned strings guarantee that equal identifiers share a single `*mut Obj`, so pointer equality is sufficient and no string hashing occurs after the intern table lookup.
- The intern table is `VecMap<String, *mut Obj>`. FNV-1a replaces Rust's default SipHash, which is designed for DoS resistance rather than speed. On tight method-dispatch benchmarks this gives a ~35% throughput improvement over the baseline.

## Remaining Optimization Opportunities

- **IP caching** — `read_byte()` walks `frames.last() → resolve_function() → chunk.codes[ip]` on every instruction. Caching `ip`, `base_pointer`, and a raw `*const u8` slice pointer as locals in `run()` eliminates that chain. In Nystrom's C, the `ip` raw pointer is register-allocated by the compiler.
- **Fixed-size stack array** — Replace `Vec<Value>` with `[Value; STACK_MAX]` and a stack pointer. Removes heap indirection and bounds-check overhead on every push/pop.
- **Interned-pointer string equality** — Global and method lookups already use pointer keys. The remaining string-content comparisons (e.g. `==` on string values) could be reduced to pointer compares once all paths are confirmed to go through the intern table.
