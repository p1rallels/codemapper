package main

import (
	"fmt"
	"os"
)

// User represents a user in the system
type User struct {
	Name  string
	Email string
	Age   int
}

// Config holds application configuration
type Config interface {
	GetPort() int
	GetHost() string
}

// main is the entry point
func main() {
	fmt.Println("Hello, World!")
}

// processUser handles user processing
func processUser(user User) error {
	fmt.Printf("Processing user: %s\n", user.Name)
	return nil
}

// GetName returns the user's name
func (u User) GetName() string {
	return u.Name
}

// UpdateEmail updates the user's email
func (u *User) UpdateEmail(email string) {
	u.Email = email
}
