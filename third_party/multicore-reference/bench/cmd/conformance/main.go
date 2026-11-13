// conformance runs a fixed, deterministic script against a Conduit server (SPEC.md) and checks
// status codes, JSON shapes and semantics. It must run against a server whose database is a FRESH copy
// of the seed (createdb -T conduit_seed / conduit_dev), because it creates users, articles and comments.
//
//	conformance -base http://127.0.0.1:3103 -secret conduit-dev-secret [-transcript out.jsonl] [-q]
//
// The transcript has one JSON line per step (step, method, path, status, normalized body). Two correct
// implementations run against fresh copies of the same seed produce byte-identical transcripts, so
// `diff golden-dev.jsonl other.jsonl` cross-checks languages. Volatile values are normalized:
// tokens -> "<token>", timestamps after the seed's end (rows written during the run) -> "<now>",
// ids of comments created during the run -> "<id>", error messages -> "<msg>".
//
// Exit status 0 only if every check passes.
package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"sort"
	"strings"
	"time"

	"conduitbench/seed"
)

type step struct {
	name, method, path string
	status             int
	raw                []byte
	fails              []string
}

type runner struct {
	base, secret string
	client       *http.Client
	steps        []*step
	names        map[string]bool
	commentIDs   map[string]bool // ids (decimal) of comments created during the run
}

func main() {
	base := flag.String("base", "http://127.0.0.1:3000", "server base URL")
	secret := flag.String("secret", os.Getenv("JWT_SECRET"), "JWT secret the server uses (default $JWT_SECRET)")
	transcript := flag.String("transcript", "", "write the normalized transcript (JSON lines) to this file")
	quiet := flag.Bool("q", false, "print only failing steps and the summary")
	hint := flag.Bool("db-reset-hint", false, "print how to give the server a fresh database, then exit")
	flag.Parse()
	if *hint {
		fmt.Println(`The script needs a FRESH copy of the seed database every run, e.g.:
  dropdb --if-exists conduit_go && createdb -T conduit_dev conduit_go   # (restart the server: pools hold connections)
Then: conformance -base http://127.0.0.1:3103 -secret conduit-dev-secret -transcript run.jsonl`)
		return
	}
	if *secret == "" {
		fmt.Fprintln(os.Stderr, "conformance: -secret (or JWT_SECRET) is required")
		os.Exit(2)
	}
	r := &runner{
		base:       strings.TrimRight(*base, "/"),
		secret:     *secret,
		client:     &http.Client{Timeout: 30 * time.Second},
		names:      map[string]bool{},
		commentIDs: map[string]bool{},
	}
	start := time.Now()
	func() {
		defer func() {
			if p := recover(); p != nil {
				if ab, ok := p.(abort); ok {
					r.steps = append(r.steps, &step{name: "ABORTED", fails: []string{string(ab)}})
					return
				}
				panic(p)
			}
		}()
		r.script()
	}()

	failed := 0
	for _, s := range r.steps {
		if len(s.fails) > 0 {
			failed++
			fmt.Printf("FAIL %-48s %s %s -> %d\n", s.name, s.method, s.path, s.status)
			for _, f := range s.fails {
				fmt.Printf("       - %s\n", f)
			}
		} else if !*quiet {
			fmt.Printf("PASS %-48s %s %s -> %d\n", s.name, s.method, s.path, s.status)
		}
	}
	if *transcript != "" {
		if err := os.WriteFile(*transcript, r.transcript(), 0o644); err != nil {
			fmt.Fprintln(os.Stderr, "write transcript:", err)
			os.Exit(2)
		}
	}
	fmt.Printf("\n%d steps, %d passed, %d failed (%.1fs)\n", len(r.steps), len(r.steps)-failed, failed, time.Since(start).Seconds())
	if failed > 0 {
		fmt.Println("RESULT: FAIL")
		os.Exit(1)
	}
	fmt.Println("RESULT: PASS")
}

// abort stops the script when a later step can't run meaningfully (e.g. login failed).
type abort string

// call is one HTTP request. body: nil (no body), []byte/string (sent verbatim) or any (JSON-encoded).
// token "" sends no Authorization header; a token starting with "raw:" sends the rest as the header value.
func (r *runner) call(name, method, path, token string, body any) *resp {
	if r.names[name] {
		panic("duplicate step name " + name)
	}
	r.names[name] = true
	var rd io.Reader
	switch b := body.(type) {
	case nil:
	case []byte:
		rd = bytes.NewReader(b)
	case string:
		rd = strings.NewReader(b)
	default:
		j, _ := json.Marshal(b)
		rd = bytes.NewReader(j)
	}
	st := &step{name: name, method: method, path: path}
	r.steps = append(r.steps, st)
	req, err := http.NewRequest(method, r.base+path, rd)
	if err != nil {
		panic(err)
	}
	if rd != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if token != "" {
		if h, ok := strings.CutPrefix(token, "raw:"); ok {
			req.Header.Set("Authorization", h)
		} else {
			req.Header.Set("Authorization", "Token "+token)
		}
	}
	res, err := r.client.Do(req)
	x := &resp{st: st}
	if err != nil {
		st.fails = append(st.fails, "request error: "+err.Error())
		return x
	}
	defer res.Body.Close()
	st.raw, _ = io.ReadAll(res.Body)
	st.status = res.StatusCode
	x.ct = res.Header.Get("Content-Type")
	if len(st.raw) > 0 {
		d := json.NewDecoder(bytes.NewReader(st.raw))
		d.UseNumber()
		if err := d.Decode(&x.v); err != nil {
			x.fail("response is not JSON: %q", trunc(st.raw))
			x.v = nil
		} else {
			x.ok = true
		}
	}
	// Generic per-status rules.
	switch {
	case st.status == 204:
		if len(st.raw) != 0 {
			x.fail("204 must have an empty body, got %q", trunc(st.raw))
		}
	case st.status >= 400 && st.status < 500:
		if !strings.HasPrefix(x.ct, "application/json") {
			x.fail("Content-Type %q, want application/json", x.ct)
		}
		checkErrors(x)
	default:
		if !strings.HasPrefix(x.ct, "application/json") {
			x.fail("Content-Type %q, want application/json", x.ct)
		}
	}
	return x
}

func trunc(b []byte) string {
	if len(b) > 200 {
		return string(b[:200]) + "..."
	}
	return string(b)
}

// ---------- transcript ----------

func (r *runner) transcript() []byte {
	var out bytes.Buffer
	ids := make([]string, 0, len(r.commentIDs))
	for id := range r.commentIDs {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, s := range r.steps {
		if s.name == "ABORTED" {
			continue
		}
		path := s.path
		for _, id := range ids {
			if strings.HasSuffix(path, "/comments/"+id) {
				path = strings.TrimSuffix(path, id) + "<id>"
			}
		}
		var body any
		if len(s.raw) > 0 {
			d := json.NewDecoder(bytes.NewReader(s.raw))
			d.UseNumber()
			if d.Decode(&body) != nil {
				body = "<invalid json>"
			} else {
				body = r.normalize(body, "")
			}
		}
		line := struct {
			Step   string `json:"step"`
			Method string `json:"method"`
			Path   string `json:"path"`
			Status int    `json:"status"`
			Body   any    `json:"body"`
		}{s.name, s.method, path, s.status, body}
		enc := json.NewEncoder(&out)
		enc.SetEscapeHTML(false)
		enc.Encode(line)
	}
	return out.Bytes()
}

func (r *runner) normalize(v any, key string) any {
	switch x := v.(type) {
	case map[string]any:
		if e, ok := x["errors"].(map[string]any); ok && key == "" {
			if _, ok := e["body"].([]any); ok {
				return map[string]any{"errors": map[string]any{"body": []any{"<msg>"}}}
			}
		}
		out := make(map[string]any, len(x))
		isComment := x["author"] != nil && x["body"] != nil && x["id"] != nil
		for k, val := range x {
			switch {
			case k == "token":
				if _, ok := val.(string); ok {
					out[k] = "<token>"
					continue
				}
			case k == "createdAt" || k == "updatedAt":
				if s, ok := val.(string); ok {
					if t, err := time.Parse(time.RFC3339Nano, s); err == nil && t.After(seed.End) {
						out[k] = "<now>"
						continue
					}
				}
			case k == "id" && isComment:
				if n, ok := val.(json.Number); ok && r.commentIDs[n.String()] {
					out[k] = "<id>"
					continue
				}
			}
			out[k] = r.normalize(val, k)
		}
		return out
	case []any:
		out := make([]any, len(x))
		for i := range x {
			out[i] = r.normalize(x[i], key)
		}
		return out
	}
	return v
}
