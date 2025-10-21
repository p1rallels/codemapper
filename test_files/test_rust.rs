use std::fs;
use std::path::Path;
use anyhow::{Context, Result};
use crate::models::Symbol;

pub struct TestStruct {
    value: i32,
}

impl TestStruct {
    pub fn new() -> Self {
        Self { value: 0 }
    }
}

pub fn test_function() {
    println!("Hello");
}
