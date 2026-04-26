use crate::vecmap::VecMap;

use crate::chunk::{Chunk, OpCode};
use crate::compiler::Compiler;
use crate::lexer::Lexer;
use crate::value::{Obj, ObjKind, Value};

use anyhow::{Result, bail};

const FRAMES_MAX: usize = 64;
const STACK_MAX: usize = FRAMES_MAX * u8::MAX as usize;
const GC_HEAP_GROWTH_FACTOR: usize = 2;

pub enum InterpretResult {
    Ok,
    CompileError,
    RuntimeError,
}

struct CallFrame {
    function: *mut Obj,
    ip: usize,
    base_pointer: usize, // index into vm.stack where this frame's locals start
}

pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<CallFrame>,

    objects: *mut Obj,

    strings: VecMap<String, *mut Obj>,
    globals: VecMap<*mut Obj, Value>,
    // consider adding
    // natives: HashMap<String, Arc<dyn Fn(&[Value]) -> Result<Value>>>
    // this would add the ability to use closures as native functions
    open_upvalues: Vec<*mut Obj>,

    toggle_tracing: bool,
    toggle_debug_print: bool,
    toggle_gc_log: bool,

    bytes_allocated: usize,
    next_gc: usize,
    grey_stack: Vec<*mut Obj>,

    init_string: *mut Obj,
}

// =============================================================================
// Lifecycle
// =============================================================================

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        let mut vm = Vm {
            stack: Vec::with_capacity(STACK_MAX),
            frames: Vec::with_capacity(FRAMES_MAX),

            objects: std::ptr::null_mut(),
            strings: VecMap::default(),
            globals: VecMap::default(),

            open_upvalues: Vec::new(),

            toggle_tracing: false,
            toggle_debug_print: false,
            toggle_gc_log: false,

            bytes_allocated: 0,
            next_gc: 1024 * 1024,
            grey_stack: Vec::new(),

            init_string: std::ptr::null_mut(),
        };

        let init_obj = vm.alloc_obj(ObjKind::String(String::from("init")));
        vm.strings.insert(String::from("init"), init_obj);
        vm.init_string = init_obj;

        vm.define_native("clock", |_args| {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();
            Ok(Value::from_number(secs))
        });

        vm.define_native("sqrt", |args| {
            if args.len() != 1 {
                bail!("'sqrt' only accepts 1 argument, got: {}", args.len());
            }
            if !args[0].is_number() {
                bail!("'sqrt' only accepts Numbers")
            };
            Ok(Value::from_number(args[0].as_number().sqrt()))
        });
        vm
    }

    pub fn with_tracing(mut self) -> Self {
        self.toggle_tracing = true;
        self
    }

    pub fn with_debug(mut self) -> Self {
        self.toggle_debug_print = true;
        self
    }

    pub fn with_gc_log(mut self) -> Self {
        self.toggle_gc_log = true;
        self
    }

    pub fn interpret(&mut self, source: &str) -> InterpretResult {
        let lexer = Lexer::from(source);
        let mut compiler = Compiler::new(lexer, &mut self.objects, &mut self.strings);
        if self.toggle_debug_print {
            compiler = compiler.with_debug();
        }

        match compiler.compile() {
            Ok(func_obj) => {
                self.stack.push(Value::from_obj(func_obj));
                let closure = self.alloc_obj(ObjKind::Closure {
                    function: func_obj,
                    upvalues: Vec::new(),
                });
                let _ = self.stack.pop();
                self.stack.push(Value::from_obj(closure));
                self.call(closure, 0);
                self.run()
            }
            Err(_) => InterpretResult::CompileError,
        }
    }

    fn reset_stack(&mut self) {
        self.frames.clear();
        self.stack.clear();
    }

    fn define_native(&mut self, name: &str, native_func: fn(&[Value]) -> Result<Value>) {
        let ptr = if let Some(&existing) = self.strings.get(name) {
            existing
        } else {
            let ptr = self.alloc_obj(ObjKind::String(name.to_string()));
            self.strings.insert(name.to_string(), ptr);
            ptr
        };
        let native_obj = self.alloc_obj(ObjKind::Native(native_func));
        self.globals.insert(ptr, Value::from_obj(native_obj));
    }

    fn alloc_obj(&mut self, kind: ObjKind) -> *mut Obj {
        self.bytes_allocated += std::mem::size_of::<Obj>() + kind.heap_size();
        // if self.toggle_gc_log {
        //     eprintln!(
        //         "allocating {} bytes for an object",
        //         std::mem::size_of::<Obj>() + kind.heap_size()
        //     );
        // }
        if self.bytes_allocated > self.next_gc {
            self.collect_garbage();
        }
        let ptr = Box::into_raw(Box::new(Obj {
            kind,
            next: self.objects,
            marked: false,
        }));
        self.objects = ptr;
        ptr
    }

    fn collect_garbage(&mut self) {
        if self.toggle_gc_log {
            eprintln!("-- gc begin");
        }
        let before = self.bytes_allocated;
        self.mark_roots();
        self.trace_references();
        self.table_remove_white();
        self.sweep();
        self.next_gc = self.bytes_allocated * GC_HEAP_GROWTH_FACTOR;
        if self.toggle_gc_log {
            eprintln!("-- gc end");
            eprintln!(
                "   collected {} bytes (from {} to {}), next at {}",
                before - self.bytes_allocated,
                before,
                self.bytes_allocated,
                self.next_gc
            );
        }
    }

    fn mark_roots(&mut self) {
        let stack_ptrs: Vec<*mut Obj> = self
            .stack
            .iter()
            .filter_map(|sv| if sv.is_obj() { Some(sv.as_obj()) } else { None })
            .collect();
        for ptr in stack_ptrs {
            self.mark_object(ptr, "Stack");
        }
        let frames_ptrs: Vec<*mut Obj> = self.frames.iter().map(|frame| frame.function).collect();
        for ptr in frames_ptrs {
            self.mark_object(ptr, "Frames");
        }
        let upvalue_ptrs: Vec<*mut Obj> = self.open_upvalues.to_vec();
        for ptr in upvalue_ptrs {
            self.mark_object(ptr, "Upvalues");
        }
        let global_ptrs: Vec<*mut Obj> = self
            .globals
            .values()
            .filter_map(|v: &Value| if v.is_obj() { Some(v.as_obj()) } else { None })
            .collect();
        for ptr in global_ptrs {
            self.mark_object(ptr, "Global");
        }
        // my design doesn't allow for directly marking roots generated
        // by the compiler. Nystrom has a `markCompilerRoots` in his C.
        // we will catch compiler allocations when we walk the memory tree
        // e.g. stack[0] → script closure → script function → chunk.constants...
    }

    fn mark_object(&mut self, obj_ptr: *mut Obj, source: &str) {
        if obj_ptr.is_null() {
            return;
        }
        let obj = unsafe { &mut *obj_ptr };
        if obj.marked {
            return;
        }
        if self.toggle_gc_log {
            println!("{source}: {} mark ", Value::from_obj(obj));
        }
        obj.marked = true;

        self.grey_stack.push(obj);
    }

    fn mark_value(&mut self, val: Value, source: &str) {
        if val.is_obj() {
            self.mark_object(val.as_obj(), source);
        }
    }

    fn trace_references(&mut self) {
        while let Some(obj) = self.grey_stack.pop() {
            self.blacken_object(obj);
        }
    }

    fn blacken_object(&mut self, obj: *mut Obj) {
        if self.toggle_gc_log {
            println!("{:?} blacken {}", obj, Value::from_obj(obj));
        }

        match unsafe { &(*obj).kind } {
            ObjKind::UpValue { location, .. } => {
                let ptr = unsafe { **location };
                if ptr.is_obj() {
                    self.mark_object(ptr.as_obj(), "Trace upvalue");
                }
            }
            ObjKind::Function { chunk, .. } => {
                chunk
                    .constants
                    .iter()
                    .filter_map(|v| if v.is_obj() { Some(v.as_obj()) } else { None })
                    .for_each(|o| self.mark_object(o, "Function chunk"));
            }
            ObjKind::Closure { function, upvalues } => {
                self.mark_object(*function, "Closure Function");
                upvalues
                    .iter()
                    .for_each(|uv| self.mark_object(*uv, "Closure Upvalue"));
            }
            ObjKind::Class { methods, .. } => {
                methods.iter().for_each(|(k, v)| {
                    self.mark_object(*k, "class method");
                    self.mark_value(*v, "class method");
                });
            }
            ObjKind::ClassInstance { klass, fields } => {
                self.mark_object(*klass, "Instance class");
                fields.iter().for_each(|(k, v)| {
                    self.mark_object(*k, "class method");
                    self.mark_value(*v, "class method");
                });
            }
            ObjKind::BoundMethod { receiver, method } => {
                self.mark_value(*receiver, "method receiver");
                self.mark_object(*method, "bound method");
            }
            _ => {}
        }
    }

    fn table_remove_white(&mut self) {
        let keys_to_remove: Vec<_> = self
            .strings
            .iter()
            .filter(|(k, v)| !k.is_empty() && !unsafe { &***v }.marked)
            .map(|(k, _)| k.clone())
            .collect();
        for k in &keys_to_remove {
            self.strings.remove(k);
        }
    }

    fn sweep(&mut self) {
        let mut previous = std::ptr::null_mut();
        let mut object = self.objects;

        while !object.is_null() {
            if unsafe { &*object }.marked {
                unsafe { &mut *object }.marked = false;
                previous = object;
                object = unsafe { &*object }.next;
            } else {
                let unreached = object;
                object = unsafe { &*object }.next;
                if !previous.is_null() {
                    unsafe { &mut *previous }.next = object;
                } else {
                    self.objects = object;
                }

                if self.toggle_gc_log {
                    println!("free: {}", Value::from_obj(unreached));
                }
                let size = size_of::<Obj>() + unsafe { &*unreached }.kind.heap_size();
                let _ = unsafe { Box::from_raw(unreached) };
                self.bytes_allocated = self.bytes_allocated.saturating_sub(size);
            }
        }
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        let mut obj = self.objects;
        while !obj.is_null() {
            // SAFETY: every Obj in the linked list was allocated with Box::into_raw.
            // We walk next before dropping so the pointer remains valid for the read.
            let next = unsafe { (*obj).next };
            drop(unsafe { Box::from_raw(obj) });
            obj = next;
        }
    }
}

// =============================================================================
// Frame
// =============================================================================

impl Vm {
    fn resolve_function(ptr: *mut Obj) -> *mut Obj {
        // SAFETY: ptr must be non-null and point to a live Obj. Callers pass either
        // CallFrame::function (set in call()) or a Value::Obj from the stack, both of
        // which originate from Box::into_raw and remain live until VM drop.
        // The Closure branch's inner function pointer is always an ObjKind::Function
        // by the invariant enforced in OpCode::Closure.
        unsafe {
            match &(*ptr).kind {
                ObjKind::Function { .. } => ptr,
                ObjKind::Closure { function, .. } => *function,
                _ => unreachable!(),
            }
        }
    }

    fn current_func(&self) -> &Obj {
        // SAFETY: resolve_function returns a valid pointer; see its SAFETY comment.
        unsafe { &*Self::resolve_function(self.frames.last().unwrap().function) }
    }

    fn current_chunk(&self) -> &Chunk {
        let ObjKind::Function { chunk, .. } = &self.current_func().kind else {
            unreachable!()
        };
        chunk
    }

    fn current_ip(&self) -> usize {
        self.frames.last().unwrap().ip
    }

    fn current_ip_mut(&mut self) -> &mut usize {
        &mut self.frames.last_mut().unwrap().ip
    }

    fn peek_stack(&self, depth: usize) -> &Value {
        &self.stack[self.stack.len() - 1 - depth]
    }

    fn read_byte(&mut self) -> u8 {
        let b = self.current_chunk().codes[self.current_ip()];
        *self.current_ip_mut() += 1;
        b
    }

    fn read_short(&mut self) -> u16 {
        let chunk = self.current_chunk();
        assert!(
            self.current_ip() < chunk.codes.len() - 1,
            "can't read two bytes when ip is so high"
        );
        let bh = chunk.codes[self.current_ip()] as u16;
        let bl = chunk.codes[self.current_ip() + 1] as u16;
        let val = bh << 8 | bl;
        *self.current_ip_mut() += 2;
        val
    }

    fn read_constant(&mut self) -> Value {
        let offset = self.read_byte() as usize;
        self.current_chunk().constants[offset]
    }
}

// =============================================================================
// Call
// =============================================================================

impl Vm {
    fn call(&mut self, func_ptr: *mut Obj, arg_count: u8) -> bool {
        // SAFETY: func_ptr comes from call_value, which extracted it from a Value::Obj
        // on the stack. All Value::Obj pointers are live Box::into_raw allocations.
        let arity = unsafe {
            match &(*func_ptr).kind {
                ObjKind::Function { arity, .. } => *arity,
                ObjKind::Closure { function, .. } => {
                    let ObjKind::Function { arity, .. } = &(**function).kind else {
                        unreachable!()
                    };
                    *arity
                }
                _ => unreachable!(),
            }
        };
        if arg_count as usize != arity {
            self.runtime_error(&format!(
                "Expected {} arguments but got {}.",
                arity, arg_count
            ));
            return false;
        }
        if self.frames.len() == FRAMES_MAX {
            self.runtime_error("Stack overflow.");
            return false;
        }
        let base_pointer = self.stack.len() - arg_count as usize - 1;
        self.frames.push(CallFrame {
            function: func_ptr,
            ip: 0,
            base_pointer,
        });
        true
    }

    fn call_value(&mut self, callee: Value, arg_count: u8) -> bool {
        if callee.is_obj()
            && let ObjKind::Native(native) = unsafe { &*callee.as_obj() }.kind
        {
            let args = &self.stack[self.stack.len() - arg_count as usize..];
            match native(args) {
                Ok(val) => {
                    self.stack
                        .truncate(self.stack.len() - arg_count as usize - 1);
                    self.stack.push(val);
                    return true;
                }
                Err(e) => {
                    self.runtime_error(&format!("Error in native function: {e}"));
                    return false;
                }
            }
        }

        if !callee.is_obj() {
            self.runtime_error("Can only call objects.");
            return false;
        }
        let ptr = callee.as_obj();

        match unsafe { &(*ptr).kind } {
            ObjKind::Function { .. } | ObjKind::Closure { .. } => self.call(ptr, arg_count),
            ObjKind::Class { .. } => {
                let instance = self.alloc_obj(ObjKind::ClassInstance {
                    klass: ptr,
                    fields: VecMap::default(),
                });
                let slot = self.stack.len() - arg_count as usize - 1;
                self.stack[slot] = Value::from_obj(instance);
                if let ObjKind::Class { methods, .. } = unsafe { &(*ptr).kind }
                    && let Some(init_val) = methods.get(&self.init_string)
                {
                    let init_ptr = init_val.as_obj();
                    return self.call(init_ptr, arg_count);
                } else if arg_count > 0 {
                    self.runtime_error(&format!("Expected 0 arguments but got {arg_count}."));
                    return false;
                }
                true
            }
            ObjKind::BoundMethod { receiver, method } => {
                let receiver = *receiver;
                let method = *method;
                let ok = self.call(method, arg_count);
                if ok {
                    let bp = self.frames.last().unwrap().base_pointer;
                    self.stack[bp] = receiver; // put the instance
                }
                ok
            }
            _ => {
                self.runtime_error("Can only call functions, closures, and classes.");
                false
            }
        }
    }

    fn invoke(&mut self, name: *mut Obj, arg_count: usize) -> bool {
        let receiver = *self.peek_stack(arg_count);
        let recv_obj = receiver.as_obj();
        let ObjKind::ClassInstance { klass, fields } = &unsafe { &*recv_obj }.kind else {
            self.runtime_error("only instances have methods.");
            return false;
        };
        if let Some(value) = fields.get(&name) {
            let bp = self.stack.len() - arg_count - 1;
            self.stack[bp] = *value;
            return self.call_value(*value, arg_count as u8);
        }
        self.invoke_from_class(*klass, name, arg_count)
    }

    fn invoke_from_class(&mut self, klass: *mut Obj, name: *mut Obj, arg_count: usize) -> bool {
        let ObjKind::Class { methods, .. } = &unsafe { &*klass }.kind else {
            unreachable!()
        };
        if let Some(closure) = methods.get(&name)
            && closure.is_obj()
        {
            let func_ptr = closure.as_obj();
            self.call(func_ptr, arg_count as u8)
        } else {
            self.runtime_error(&format!("Undefined property '{}'", Value::from_obj(name)));
            false
        }
    }

    fn capture_upvalue(&mut self, slot: usize) -> *mut Obj {
        let base = self.frames.last().unwrap().base_pointer;
        let location = &mut self.stack[base + slot] as *mut Value;

        // reuse existing open upvalue for this stack slot if one exists
        for &uv_ptr in &self.open_upvalues {
            // SAFETY: open_upvalues contains live ObjKind::UpValue allocations.
            if let ObjKind::UpValue { location: loc, .. } = unsafe { &(*uv_ptr).kind }
                && *loc == location
            {
                return uv_ptr;
            }
        }

        let upvalue = self.alloc_obj(ObjKind::UpValue {
            location,
            closed: Value::nil(),
        });
        self.open_upvalues.push(upvalue);
        upvalue
    }
}

// =============================================================================
// Execution
// =============================================================================

impl Vm {
    pub fn run(&mut self) -> InterpretResult {
        // TODO: Cache hot state as locals — avoids frame chain walk per instruction
        loop {
            if self.toggle_tracing {
                print!("      ");
                self.stack.iter().for_each(|val| print!("[{val}]"));
                println!();
                self.current_chunk()
                    .disassemble_instruction(self.current_ip());
            }
            match OpCode::from(self.read_byte()) {
                OpCode::Jump => {
                    let offset = self.read_short() as usize;
                    *self.current_ip_mut() += offset;
                }
                OpCode::JumpIfFalse => {
                    let offset = self.read_short() as usize;
                    if self.peek_stack(0).is_falsey() {
                        *self.current_ip_mut() += offset;
                    }
                }
                OpCode::Loop => {
                    let offset = self.read_short() as usize;
                    *self.current_ip_mut() -= offset;
                }
                OpCode::Call => {
                    let arg_count = self.read_byte();
                    let callee = *self.peek_stack(arg_count as usize);
                    if !self.call_value(callee, arg_count) {
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::Invoke => {
                    let method_val = self.read_constant();
                    let method_ptr = method_val.as_obj();
                    let arg_count = self.read_byte() as usize;
                    if !self.invoke(method_ptr, arg_count) {
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::SuperInvoke => {
                    let method_val = self.read_constant();
                    let method_ptr = method_val.as_obj();
                    let arg_count = self.read_byte() as usize;
                    let super_val = self.stack.pop().unwrap();

                    let super_ptr = super_val.as_obj();
                    if !self.invoke_from_class(super_ptr, method_ptr, arg_count) {
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::Closure => {
                    let ptr = self.read_constant().as_obj();
                    // SAFETY: ptr came from read_constant, which copied it from the
                    // constants pool — a live Box::into_raw allocation.
                    assert!(matches!(&unsafe { &*ptr }.kind, ObjKind::Function { .. }));

                    let upvalue_count = unsafe {
                        let ObjKind::Function { upvalue_count, .. } = &(*ptr).kind else {
                            unreachable!()
                        };
                        *upvalue_count
                    };
                    let mut upvalues = Vec::with_capacity(upvalue_count);
                    for _ in 0..upvalue_count {
                        let is_local = self.read_byte() != 0;
                        let index = self.read_byte() as usize;
                        if is_local {
                            upvalues.push(self.capture_upvalue(index));
                        } else {
                            // reuse upvalue from enclosing closure
                            let enclosing = self.frames.last().unwrap().function;
                            // SAFETY: enclosing frame's function is a live Closure obj.
                            let ObjKind::Closure {
                                upvalues: enc_upvalues,
                                ..
                            } = (unsafe { &(*enclosing).kind })
                            else {
                                unreachable!()
                            };
                            upvalues.push(enc_upvalues[index]);
                        }
                    }

                    let closure = self.alloc_obj(ObjKind::Closure {
                        function: ptr,
                        upvalues,
                    });
                    self.stack.push(Value::from_obj(closure));
                }
                OpCode::CloseUpvalue => {
                    self.close_upvalues(self.stack.len() - 1);
                    self.stack.pop();
                }
                OpCode::Return => {
                    let result = self.stack.pop().unwrap();
                    let bp = self.frames.pop().unwrap().base_pointer;
                    if self.frames.is_empty() {
                        self.stack.pop(); // pop the script function
                        return InterpretResult::Ok;
                    }
                    self.close_upvalues(bp);
                    self.stack.truncate(bp);
                    self.stack.push(result);
                }
                OpCode::Constant => {
                    let constant = self.read_constant();
                    self.stack.push(constant);
                }
                OpCode::Nil => self.stack.push(Value::nil()),
                OpCode::True => self.stack.push(Value::from_bool(true)),
                OpCode::False => self.stack.push(Value::from_bool(false)),
                OpCode::Pop => {
                    self.stack.pop();
                }
                OpCode::GetLocal => {
                    let slot = self.read_byte() as usize;
                    let base = self.frames.last().unwrap().base_pointer;
                    self.stack.push(self.stack[base + slot]);
                }
                OpCode::SetLocal => {
                    let slot = self.read_byte() as usize;
                    let base = self.frames.last().unwrap().base_pointer;
                    self.stack[base + slot] = *self.peek_stack(0);
                }
                OpCode::DefineGlobal => {
                    let const_val = self.read_constant();
                    let key = const_val.as_obj();
                    self.globals.insert(key, *self.peek_stack(0));
                    self.stack.pop();
                }
                OpCode::GetGlobal => {
                    let const_val = self.read_constant();
                    let key = const_val.as_obj();
                    if let Some(val) = self.globals.get(&key) {
                        self.stack.push(*val);
                    } else {
                        self.runtime_error(&format!("Undefined variable '{}'", const_val));
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::SetGlobal => {
                    let const_val = self.read_constant();
                    let key = const_val.as_obj();
                    if self.globals.contains_key(&key) {
                        self.globals.insert(key, *self.peek_stack(0));
                    } else {
                        self.runtime_error(&format!("Undefined variable '{}'", const_val));
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::GetProperty => {
                    let const_val = self.read_constant();
                    let key = const_val.as_obj();
                    let instance = *self.peek_stack(0);
                    if !instance.is_obj() {
                        self.runtime_error("Only instances have properties.");
                        return InterpretResult::RuntimeError;
                    };
                    let inst_ptr = instance.as_obj();
                    let ObjKind::ClassInstance { ref fields, klass } = unsafe { &*inst_ptr }.kind
                    else {
                        self.runtime_error("Only instances have properties.");
                        return InterpretResult::RuntimeError;
                    };
                    if let Some(&val) = fields.get(&key) {
                        self.stack.pop();
                        self.stack.push(val);
                    } else if !self.bind_method(klass, key) {
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::SetProperty => {
                    let instance = *self.peek_stack(1);
                    let const_val = self.read_constant();
                    let key = const_val.as_obj();
                    let inst_ptr = instance.as_obj();
                    let ObjKind::ClassInstance { ref mut fields, .. } =
                        unsafe { &mut *inst_ptr }.kind
                    else {
                        self.runtime_error("Only instances have properties.");
                        return InterpretResult::RuntimeError;
                    };
                    let is_new = !fields.contains_key(&key);
                    fields.insert(key, *self.peek_stack(0));
                    if is_new {
                        self.bytes_allocated += size_of::<*mut Obj>() + size_of::<Value>();
                    }
                    let value = self.stack.pop().unwrap();
                    let _ = self.stack.pop();
                    self.stack.push(value);
                }
                OpCode::Equal => {
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(Value::from_bool(x == y));
                }
                OpCode::GetUpValue => {
                    let slot = self.read_byte() as usize;
                    let func_ptr = self.frames.last().unwrap().function;
                    // SAFETY: frame.function is a live Closure; upvalues[slot] is a live upvalue.
                    let val = unsafe {
                        let ObjKind::Closure { upvalues, .. } = &(*func_ptr).kind else {
                            unreachable!()
                        };
                        let uv_ptr = upvalues[slot];
                        let ObjKind::UpValue { location, .. } = &(*uv_ptr).kind else {
                            unreachable!()
                        };
                        **location
                    };
                    self.stack.push(val);
                }
                OpCode::SetUpValue => {
                    let slot = self.read_byte() as usize;
                    let val = *self.peek_stack(0);
                    let func_ptr = self.frames.last().unwrap().function;
                    // SAFETY: same as GetUpValue.
                    unsafe {
                        let ObjKind::Closure { upvalues, .. } = &(*func_ptr).kind else {
                            unreachable!()
                        };
                        let uv_ptr = upvalues[slot];
                        let ObjKind::UpValue { location, .. } = &(*uv_ptr).kind else {
                            unreachable!()
                        };
                        **location = val;
                    };
                }
                OpCode::GetSuper => {
                    let const_val = self.read_constant();
                    let key = const_val.as_obj();
                    let super_val = self.stack.pop().unwrap();
                    let super_ptr = super_val.as_obj();
                    assert!(matches!(
                        &unsafe { &*super_ptr }.kind,
                        ObjKind::Class { .. }
                    ));
                    // using bind_method with the super class as opposed to the instance's class
                    if !self.bind_method(super_ptr, key) {
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::Greater => {
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(Value::from_bool(x > y));
                }
                OpCode::Less => {
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(Value::from_bool(x < y));
                }
                OpCode::Negate => {
                    if !self.peek_stack(0).is_number() {
                        self.runtime_error("operand must be a number");
                        return InterpretResult::RuntimeError;
                    }
                    let popped = self.stack.pop().unwrap();
                    self.stack.push(-popped);
                }
                OpCode::Add => {
                    let both_numbers =
                        self.peek_stack(0).is_number() && self.peek_stack(1).is_number();
                    let both_objects = self.peek_stack(0).is_obj() && self.peek_stack(1).is_obj();
                    if both_numbers {
                        let y = self.stack.pop().unwrap();
                        let x = self.stack.pop().unwrap();
                        self.stack.push(x + y);
                    } else if both_objects {
                        if !self.concatenate() {
                            self.runtime_error("operands must be two strings");
                            return InterpretResult::RuntimeError;
                        }
                    } else {
                        self.runtime_error("operands must be numbers or strings");
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::Subtract => {
                    if !(self.peek_stack(0).is_number() && self.peek_stack(1).is_number()) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x - y);
                }
                OpCode::Multiply => {
                    if !(self.peek_stack(0).is_number() && self.peek_stack(1).is_number()) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x * y);
                }
                OpCode::Divide => {
                    if !(self.peek_stack(0).is_number() && self.peek_stack(1).is_number()) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x / y);
                }
                OpCode::Not => {
                    let top = self.stack.pop().unwrap();
                    self.stack.push(Value::from_bool(top.is_falsey()));
                }
                OpCode::Print => {
                    println!("{}", self.stack.pop().unwrap());
                }
                OpCode::Class => {
                    let thing = self.read_constant();
                    let ptr = thing.as_obj();
                    let ObjKind::String(name) = &(unsafe { &*ptr }.kind) else {
                        unreachable!()
                    };
                    let ptr = self.alloc_obj(ObjKind::Class {
                        name: name.clone(),
                        methods: VecMap::default(),
                    });
                    self.stack.push(Value::from_obj(ptr));
                }
                OpCode::Method => {
                    let method_val = self.read_constant();
                    let method_ptr = method_val.as_obj();
                    self.define_method(method_ptr);
                }
                OpCode::Inherit => {
                    let super_class = *self.peek_stack(1);
                    let sub_class = *self.peek_stack(0);
                    if !super_class.is_obj() {
                        self.runtime_error("Superclass must be a class.");
                        return InterpretResult::RuntimeError;
                    }
                    let super_ptr = super_class.as_obj();
                    let sub_ptr = sub_class.as_obj();
                    let ObjKind::Class {
                        methods: super_methods,
                        ..
                    } = &unsafe { &*super_ptr }.kind
                    else {
                        unreachable!()
                    };
                    let ObjKind::Class {
                        methods: sub_methods,
                        ..
                    } = &mut unsafe { &mut *sub_ptr }.kind
                    else {
                        unreachable!()
                    };
                    sub_methods.extend(super_methods);
                    let _ = self.stack.pop(); // pops the subclass off the vm stack
                }
            }
        }
    }

    fn concatenate(&mut self) -> bool {
        if !(self.peek_stack(0).is_obj() && self.peek_stack(1).is_obj()) {
            return false;
        }
        let (p1, p2) = (self.peek_stack(0).as_obj(), self.peek_stack(1).as_obj());
        // SAFETY: p1 and p2 were copied from Value::Obj entries on the stack,
        // both of which are live Box::into_raw allocations.
        let is_strings = unsafe {
            matches!(
                (&(*p1).kind, &(*p2).kind),
                (ObjKind::String(_), ObjKind::String(_))
            )
        };
        if !is_strings {
            return false;
        }

        let ptr2 = self.peek_stack(0).as_obj();
        let ptr1 = self.peek_stack(1).as_obj();
        // SAFETY: ptr1/ptr2 were just popped from the stack; is_strings confirmed they
        // are live string objects. No other references exist after the pop.
        let (obj1, obj2) = unsafe { (&*ptr1, &*ptr2) };
        match (&obj1.kind, &obj2.kind) {
            (ObjKind::String(s1), ObjKind::String(s2)) => {
                let result = s1.clone() + s2.as_str();
                if let Some(&ptr) = self.strings.get(&result) {
                    let _ = self.stack.pop(); // pop ptr2 off the stack
                    let _ = self.stack.pop(); // pop ptr1 off the stack
                    self.stack.push(Value::from_obj(ptr));
                    return true;
                }
                let ptr = self.alloc_obj(ObjKind::String(result.clone()));

                self.strings.insert(result, ptr);
                let _ = self.stack.pop(); // pop ptr2 off the stack
                let _ = self.stack.pop(); // pop ptr1 off the stack
                self.stack.push(Value::from_obj(ptr));
            }
            _ => panic!("invalid object types!"),
        }
        true
    }

    fn runtime_error(&mut self, msg: &str) {
        eprintln!("{msg}");
        for frame in self.frames.iter().rev() {
            let func_ptr = Self::resolve_function(frame.function);
            // SAFETY: resolve_function returns a valid Function pointer; see its SAFETY comment.
            let ObjKind::Function { name, chunk, .. } = &unsafe { &*func_ptr }.kind else {
                unreachable!()
            };
            let line = chunk.lines[frame.ip - 1];
            if name.is_empty() {
                eprintln!("[line {line}] in script");
            } else {
                eprintln!("[line {line}] in {name}()");
            }
        }
        self.reset_stack();
    }

    fn close_upvalues(&mut self, base: usize) {
        let base_ptr = &self.stack[base] as *const Value;
        self.open_upvalues.retain(|&uv_ptr| {
            // SAFETY: open_upvalues contains live ObjKind::UpValue allocations.
            let ObjKind::UpValue { location, closed } = (unsafe { &mut (*uv_ptr).kind }) else {
                unreachable!()
            };
            if *location as *const Value >= base_ptr {
                *closed = unsafe { **location };
                *location = closed as *mut Value;
                false // remove from open_values
            } else {
                true // keep in open_values
            }
        })
    }

    fn define_method(&mut self, name_ptr: *mut Obj) {
        let method_val = self.peek_stack(0);
        let klass_val = self.peek_stack(1);

        let method_ptr = method_val.as_obj();
        let klass_ptr = klass_val.as_obj();

        if let ObjKind::Closure { .. } = unsafe { &(*method_ptr).kind }
            && let ObjKind::Class { methods, .. } = unsafe { &mut (*klass_ptr).kind }
        {
            methods.insert(name_ptr, *method_val);
        } else {
            self.runtime_error("method declarations must be functions");
        }

        self.stack.pop();
    }

    fn bind_method(&mut self, klass: *mut Obj, key: *mut Obj) -> bool {
        let ObjKind::Class { methods, .. } = &unsafe { &*klass }.kind else {
            unreachable!()
        };
        if let Some(method) = methods.get(&key).copied() {
            let receiver = *self.peek_stack(0);
            let method_ptr = method.as_obj();
            let ptr = self.alloc_obj(ObjKind::BoundMethod {
                receiver,
                method: method_ptr,
            });
            let _ = self.stack.pop(); // pop the instance off the stack
            self.stack.push(Value::from_obj(ptr));
            true
        } else {
            self.runtime_error(&format!("undefined property '{}'.", Value::from_obj(key)));
            false
        }
    }
}
