#include <stdio.h>
#include <stdlib.h>
#include "myheader.h"

// User structure definition
struct User {
    char name[50];
    int age;
    char email[100];
};

// Point union for coordinates
union Point {
    int coords[2];
    struct {
        int x;
        int y;
    };
};

typedef struct {
    int id;
    char name[100];
} Employee;

// main function - entry point
int main(void) {
    printf("Hello, World!\n");
    return 0;
}

// processUser handles user data
int processUser(struct User *user) {
    printf("Processing user: %s\n", user->name);
    return 0;
}

// calculateSum adds two numbers
int calculateSum(int a, int b) {
    return a + b;
}

// printArray displays array contents
void printArray(int arr[], int size) {
    for (int i = 0; i < size; i++) {
        printf("%d ", arr[i]);
    }
    printf("\n");
}
