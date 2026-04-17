use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone)]
pub enum Value {
    Number(f64),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{n}"),
        }
    }
}

impl Neg for Value {
    type Output = Value;
    fn neg(self) -> Self::Output {
        match self {
            Value::Number(n) => Value::Number(-n),
        }
    }
}

impl Add<Value> for Value {
    type Output = Value;
    fn add(self, rhs: Value) -> Self::Output {
        match (self, rhs) {
            (Value::Number(x), Value::Number(y)) => Value::Number(x + y),
        }
    }
}

impl Sub<Value> for Value {
    type Output = Value;
    fn sub(self, rhs: Value) -> Self::Output {
        match (self, rhs) {
            (Value::Number(x), Value::Number(y)) => Value::Number(x - y),
        }
    }
}

impl Mul<Value> for Value {
    type Output = Value;
    fn mul(self, rhs: Value) -> Self::Output {
        match (self, rhs) {
            (Value::Number(x), Value::Number(y)) => Value::Number(x * y),
        }
    }
}

impl Div<Value> for Value {
    type Output = Value;
    fn div(self, rhs: Value) -> Self::Output {
        match (self, rhs) {
            (Value::Number(x), Value::Number(y)) => Value::Number(x / y),
        }
    }
}
