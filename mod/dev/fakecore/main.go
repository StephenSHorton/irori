// Command fakecore is a stand-in Irori Core for the mod's live checks. It
// serves the v0.2 routes the mod uses (internal/api/routes_v02.go) with canned
// answers and logs every request, headers included, as JSON lines to
// fakecore.log in its working directory.
//
//	fakecore [addr]                         default 127.0.0.1:47811
//	POST /ctl/deliver?session=&text=        push a deliver event to an open stream
//	POST /ctl/event?session=   <json>       push any event
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"
)

const (
	token  = "faketoken123"
	secret = "attachsecret456"
)

var (
	mu      sync.Mutex
	logf    *os.File
	streams = map[string]chan string{} // session -> event lines
	policy  = map[string]any{"mode": "everyone", "block": []string{}, "allow": []string{}, "paused": false}
	wakeN   = 0
	addr    = "127.0.0.1:47811"
)

func record(r *http.Request, body string) {
	h := map[string]string{}
	for k, v := range r.Header {
		h[k] = strings.Join(v, ",")
	}
	_, hasOrigin := r.Header["Origin"]
	line, _ := json.Marshal(map[string]any{
		"t": time.Now().Format(time.RFC3339Nano), "method": r.Method, "url": r.URL.String(),
		"headers": h, "hasOrigin": hasOrigin, "body": body,
	})
	mu.Lock()
	logf.Write(append(line, '\n'))
	mu.Unlock()
}

func send(session string, ev map[string]any) bool {
	b, _ := json.Marshal(ev)
	mu.Lock()
	ch := streams[session]
	if ch == nil {
		for _, c := range streams { // any open stream
			ch = c
		}
	}
	mu.Unlock()
	if ch == nil {
		return false
	}
	ch <- string(b)
	return true
}

func deliver(session, text string) string {
	mu.Lock()
	wakeN++
	id := fmt.Sprintf("wake_FAKE%d_%d", time.Now().Unix(), wakeN)
	mu.Unlock()
	tok := "[irori-wake:" + id + "]"
	prompt := fmt.Sprintf("%s\nIrori message (fake Core test).\n<irori-message from=\"tester\" key=\"dev/test\" delivery=\"del_%s\" trust=\"untrusted\">%s</irori-message>", tok, id, text)
	if !send(session, map[string]any{"type": "deliver", "wakeId": id, "token": tok, "deliveryIds": []string{"del_" + id},
		"messageIds": []string{"msg_" + id}, "prompt": prompt, "from": "tester"}) {
		return "no stream"
	}
	return tok
}

func main() {
	if len(os.Args) > 1 {
		addr = os.Args[1]
	}
	var err error
	logf, err = os.OpenFile("fakecore.log", os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		log.Fatal(err)
	}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /v1/attach", func(w http.ResponseWriter, r *http.Request) {
		record(r, "")
		session := r.URL.Query().Get("session")
		ch := make(chan string, 32)
		mu.Lock()
		if old := streams[session]; old != nil {
			old <- `{"type":"superseded"}`
		}
		streams[session] = ch
		mu.Unlock()
		w.Header().Set("content-type", "application/x-ndjson")
		fl := w.(http.Flusher)
		hello, _ := json.Marshal(map[string]any{"type": "hello", "v": 1, "session": session, "agentKey": "dev/" + session,
			"name": "fake-session", "api": map[string]string{"url": "http://" + addr, "token": token}, "attachSecret": secret})
		fmt.Fprintf(w, "%s\n\n", hello)
		fmt.Fprint(w, `{"type":"inbox","unread":2,"held":1}`+"\n")
		fl.Flush()
		ka := time.NewTicker(15 * time.Second)
		defer ka.Stop()
		for {
			select {
			case <-r.Context().Done():
				mu.Lock()
				if streams[session] == ch {
					delete(streams, session)
				}
				mu.Unlock()
				return
			case <-ka.C:
				fmt.Fprint(w, "\n")
				fl.Flush()
			case line := <-ch:
				// Two writes per line, to exercise the mod's partial-line buffering.
				half := len(line) / 2
				fmt.Fprint(w, line[:half])
				fl.Flush()
				time.Sleep(50 * time.Millisecond)
				fmt.Fprint(w, line[half:]+"\n")
				fl.Flush()
				if strings.Contains(line, `"superseded"`) {
					return
				}
			}
		}
	})
	mux.HandleFunc("GET /v1/tools", func(w http.ResponseWriter, r *http.Request) {
		record(r, "")
		w.Header().Set("content-type", "application/json")
		fmt.Fprint(w, `[{"name":"echo","description":"Irori fake: echoes its args back.","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]},"readOnly":true,"clients":["claude"]},`+
			`{"name":"agents","description":"Irori fake: list agents.","inputSchema":{"type":"object","properties":{}},"readOnly":true,"clients":["claude","grok"]}]`)
	})
	mux.HandleFunc("POST /v1/tools/{name}", func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		record(r, string(b))
		if r.Header.Get("X-Irori-Attach") != secret || r.Header.Get("Authorization") != "Bearer "+token {
			w.WriteHeader(403)
			fmt.Fprint(w, `{"error":"no attach"}`)
			return
		}
		out, _ := json.Marshal(map[string]string{"text": "fake " + r.PathValue("name") + " got " + string(b)})
		w.Write(out)
	})
	mux.HandleFunc("POST /v1/deliveries/{what}", func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		record(r, string(b))
		fmt.Fprint(w, `{"ok":true}`)
	})
	mux.HandleFunc("GET /v1/policy", func(w http.ResponseWriter, r *http.Request) {
		record(r, "")
		mu.Lock()
		b, _ := json.Marshal(policy)
		mu.Unlock()
		w.Write(b)
	})
	mux.HandleFunc("POST /v1/policy", func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		record(r, string(b))
		if r.Header.Get("X-Irori-Origin") != "composer" {
			w.WriteHeader(403)
			fmt.Fprint(w, `{"error":"needs composer"}`)
			return
		}
		var p map[string]any
		json.Unmarshal(b, &p)
		mu.Lock()
		policy = p
		mu.Unlock()
		w.Write(b)
	})
	mux.HandleFunc("POST /ctl/deliver", func(w http.ResponseWriter, r *http.Request) {
		record(r, "")
		text := r.URL.Query().Get("text")
		if text == "" {
			text = "Reply with exactly: irori-fake-pong"
		}
		fmt.Fprint(w, deliver(r.URL.Query().Get("session"), text))
	})
	mux.HandleFunc("POST /ctl/event", func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		record(r, string(b))
		var ev map[string]any
		json.Unmarshal(b, &ev)
		fmt.Fprint(w, send(r.URL.Query().Get("session"), ev))
	})
	log.Printf("fake core on %s", addr)
	log.Fatal(http.ListenAndServe(addr, mux))
}
