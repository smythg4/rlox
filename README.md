# rlox

A Rust implementation of the Lox bytecode VM, following Part III of Bob Nystrom's [*Crafting Interpreters*](https://craftinginterpreters.com).

Lox is a dynamically typed scripting language. clox is its bytecode-compiled implementation written in C. This is that, but in Rust so I called it rlox. Very original, no?

## Example

```lox
fun factorial(n) {
  if (n <= 1) return 1;
  return n * factorial(n - 1);
}

fun first_over(limit) {
  var n = 0;
  while (true) {
    n = n + 1;
    if (n * n > limit) return n;
  }
}

print factorial(5);
print factorial(10);
print first_over(20);
print first_over(100);

var w = 0;
while (w < 6) {
  for (var f = 10; f < 14 and w < 3; f = f + 1) {
    print f;
  }
  w = w + 1;
  print w;
}
```

```
120
3628800
5
11
10
11
12
13
1
10
11
12
13
2
10
11
12
13
3
4
5
6
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

**In progress**

- Closures
- Classes and inheritance
- Mark-and-sweep garbage collection (linked list infrastructure is in place)
