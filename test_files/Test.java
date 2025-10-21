package com.example.test;

import java.util.List;
import java.util.ArrayList;

/**
 * A simple calculator class for demonstration
 */
public class Calculator {
    private int value;

    /**
     * Constructor for Calculator
     */
    public Calculator() {
        this.value = 0;
    }

    /**
     * Adds a number to the current value
     * @param x the number to add
     * @return the new value
     */
    public int add(int x) {
        this.value += x;
        return this.value;
    }

    /**
     * Subtracts a number from the current value
     * @param x the number to subtract
     * @return the new value
     */
    public int subtract(int x) {
        this.value -= x;
        return this.value;
    }

    /**
     * Gets the current value
     * @return the current value
     */
    public int getValue() {
        return this.value;
    }
}

/**
 * Interface for mathematical operations
 */
interface MathOperations {
    int calculate(int a, int b);
}
