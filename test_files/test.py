import os
import sys
from pathlib import Path

class Calculator:
    """A simple calculator class"""
    
    def __init__(self):
        self.value = 0
    
    def add(self, x):
        """Add a number"""
        self.value += x
        return self.value
    
    def subtract(self, x):
        """Subtract a number"""
        self.value -= x
        return self.value

def standalone_function(a, b):
    """A standalone function"""
    return a + b

def another_function():
    """Another function"""
    pass
