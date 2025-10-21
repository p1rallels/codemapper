"""A sample Python module for testing"""

import os
import sys
from pathlib import Path

def simple_function(x, y):
    """Add two numbers"""
    return x + y

class Calculator:
    """A simple calculator class"""
    
    def __init__(self, name):
        self.name = name
    
    def add(self, a, b):
        """Add two numbers"""
        return a + b
    
    def subtract(self, a, b):
        """Subtract two numbers"""
        return a - b

class ScientificCalculator(Calculator):
    """An advanced calculator"""
    
    def power(self, base, exp):
        """Calculate power"""
        return base ** exp
