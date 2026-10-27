// Command fakeirori stands in for `irori attach` in the mod's live checks: it
// dials the fake Core's /v1/attach (FAKE_CORE, default http://127.0.0.1:47811)
// and copies the stream to stdout, exiting 0 on superseded and 1 when the
// stream ends. It appends its argv to fakeirori.log beside the executable.
package main

import (
	"bufio"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func main() {
	exe, _ := os.Executable()
	lf, _ := os.OpenFile(filepath.Join(filepath.Dir(exe), "fakeirori.log"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
	fmt.Fprintf(lf, "%s argv=%q\n", time.Now().Format(time.RFC3339Nano), os.Args)
	if len(os.Args) < 2 || os.Args[1] != "attach" {
		fmt.Fprintln(os.Stderr, "fakeirori: only attach")
		os.Exit(2)
	}
	var session, mod string
	for i := 2; i+1 < len(os.Args); i++ {
		switch os.Args[i] {
		case "--session":
			session = os.Args[i+1]
		case "--mod-version":
			mod = os.Args[i+1]
		}
	}
	core := os.Getenv("FAKE_CORE")
	if core == "" {
		core = "http://127.0.0.1:47811"
	}
	req, _ := http.NewRequest("GET", core+"/v1/attach?session="+url.QueryEscape(session)+"&mod="+url.QueryEscape(mod), nil)
	req.Header.Set("Authorization", "Bearer faketoken123")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		fmt.Printf(`{"type":"error","message":%q}`+"\n", err.Error())
		os.Exit(1)
	}
	defer resp.Body.Close()
	r := bufio.NewReader(resp.Body)
	for {
		line, err := r.ReadString('\n')
		if line != "" {
			os.Stdout.WriteString(line)
			if strings.Contains(line, `"superseded"`) {
				fmt.Fprintf(lf, "%s superseded, exit 0\n", time.Now().Format(time.RFC3339Nano))
				os.Exit(0)
			}
		}
		if err != nil {
			fmt.Fprintf(lf, "%s stream ended: %v\n", time.Now().Format(time.RFC3339Nano), err)
			os.Exit(1)
		}
	}
}
