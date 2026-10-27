package proc

import (
	"os"
	"testing"
	"time"
)

func TestSelfIsAlive(t *testing.T) {
	if !Alive(os.Getpid()) {
		t.Fatal("own pid reported dead")
	}
	if Alive(0) || Alive(-1) {
		t.Fatal("non-positive pid reported alive")
	}
}

func TestSelfStartTime(t *testing.T) {
	st, ok := StartTime(os.Getpid())
	if !ok {
		t.Skip("start time not supported on this OS")
	}
	if age := time.Since(st); age < 0 || age > time.Hour {
		t.Fatalf("implausible start time %v (age %v)", st, age)
	}
}
