package api

import (
	"context"
	"net"
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/discovery"
)

// A transport failure on a member stream must not echo the member secret
// (it rides in the query) into the error text.
func TestOpenStreamErrorHidesQuery(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			c.Close() // EOF before any response
		}
	}()
	c := &Client{impl: &client{
		lf:   discovery.Lockfile{Token: "tok"},
		base: "http://" + ln.Addr().String(),
		http: &http.Client{},
	}}
	_, err = c.OpenStream(context.Background(), url.Values{"member": {"SECRET123"}})
	if err == nil {
		t.Fatal("OpenStream against a closing listener succeeded")
	}
	if strings.Contains(err.Error(), "SECRET123") {
		t.Fatalf("error leaks the member secret: %v", err)
	}
	if !strings.Contains(err.Error(), "/v1/attach") {
		t.Fatalf("error lost its context: %v", err)
	}
}
