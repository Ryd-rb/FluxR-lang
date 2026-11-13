package main

import (
	"encoding/json"
	"fmt"
	"regexp"
	"sort"
	"strings"
)

type resp struct {
	st *step
	v  any
	ok bool // body parsed as JSON
	ct string
}

func (x *resp) fail(f string, a ...any) { x.st.fails = append(x.st.fails, fmt.Sprintf(f, a...)) }

// expect records a failure unless the status is want; it reports whether it matched.
func (x *resp) expect(want int) bool {
	if x.st.status != want {
		x.fail("status %d, want %d (body %s)", x.st.status, want, trunc(x.st.raw))
		return false
	}
	return true
}

// at walks objects by key; it returns nil when any step is missing.
func at(v any, keys ...string) any {
	for _, k := range keys {
		m, ok := v.(map[string]any)
		if !ok {
			return nil
		}
		v = m[k]
	}
	return v
}

func (x *resp) at(keys ...string) any { return at(x.v, keys...) }

func str(v any) string {
	s, _ := v.(string)
	return s
}

// num returns an integer JSON number, or -1 (and ok=false) when v isn't one.
func num(v any) (int64, bool) {
	n, ok := v.(json.Number)
	if !ok || strings.ContainsAny(n.String(), ".eE") {
		return -1, false
	}
	i, err := n.Int64()
	return i, err == nil
}

func arr(v any) []any {
	a, _ := v.([]any)
	return a
}

var tsRe = regexp.MustCompile(`^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$`)

// keys checks that v is an object with exactly the given keys.
func keys(x *resp, where string, v any, want ...string) (map[string]any, bool) {
	m, ok := v.(map[string]any)
	if !ok {
		x.fail("%s: want an object, got %s", where, show(v))
		return nil, false
	}
	have := make([]string, 0, len(m))
	for k := range m {
		have = append(have, k)
	}
	sort.Strings(have)
	w := append([]string(nil), want...)
	sort.Strings(w)
	if strings.Join(have, ",") != strings.Join(w, ",") {
		x.fail("%s: keys [%s], want [%s]", where, strings.Join(have, ","), strings.Join(w, ","))
		return m, false
	}
	return m, true
}

func show(v any) string {
	b, _ := json.Marshal(v)
	return trunc(b)
}

func isString(x *resp, where string, v any, nonEmpty bool) {
	s, ok := v.(string)
	if !ok {
		x.fail("%s: want a string, got %s", where, show(v))
	} else if nonEmpty && s == "" {
		x.fail("%s: empty string", where)
	}
}

func isNullableString(x *resp, where string, v any) {
	if v == nil {
		return
	}
	if _, ok := v.(string); !ok {
		x.fail("%s: want a string or null, got %s", where, show(v))
	}
}

func isBool(x *resp, where string, v any) {
	if _, ok := v.(bool); !ok {
		x.fail("%s: want a boolean, got %s", where, show(v))
	}
}

func isTimestamp(x *resp, where string, v any) {
	s, ok := v.(string)
	if !ok || !tsRe.MatchString(s) {
		x.fail("%s: want a timestamp like 2025-04-01T12:00:00.000Z, got %s", where, show(v))
	}
}

func checkErrors(x *resp) {
	m, ok := keys(x, "error", x.v, "errors")
	if !ok {
		return
	}
	e, ok := keys(x, "errors", m["errors"], "body")
	if !ok {
		return
	}
	b := arr(e["body"])
	if len(b) == 0 {
		x.fail("errors.body: want a non-empty array of strings, got %s", show(e["body"]))
	}
	for _, s := range b {
		isString(x, "errors.body[]", s, false)
	}
}

func checkUser(x *resp, v any) {
	m, ok := keys(x, "user", v, "email", "token", "username", "bio", "image")
	if !ok {
		return
	}
	isString(x, "user.email", m["email"], true)
	isString(x, "user.token", m["token"], true)
	isString(x, "user.username", m["username"], true)
	isNullableString(x, "user.bio", m["bio"])
	isNullableString(x, "user.image", m["image"])
}

func checkProfile(x *resp, where string, v any) {
	m, ok := keys(x, where, v, "username", "bio", "image", "following")
	if !ok {
		return
	}
	isString(x, where+".username", m["username"], true)
	isNullableString(x, where+".bio", m["bio"])
	isNullableString(x, where+".image", m["image"])
	isBool(x, where+".following", m["following"])
}

func checkArticle(x *resp, where string, v any, withBody bool) {
	want := []string{"slug", "title", "description", "tagList", "createdAt", "updatedAt", "favorited", "favoritesCount", "author"}
	if withBody {
		want = append(want, "body")
	}
	m, ok := keys(x, where, v, want...)
	if !ok {
		return
	}
	isString(x, where+".slug", m["slug"], true)
	isString(x, where+".title", m["title"], true)
	isString(x, where+".description", m["description"], true)
	if withBody {
		isString(x, where+".body", m["body"], true)
	}
	tl, isArr := m["tagList"].([]any)
	if !isArr {
		x.fail("%s.tagList: want an array, got %s", where, show(m["tagList"]))
	}
	prev := ""
	for i, t := range tl {
		s, ok := t.(string)
		if !ok || s == "" {
			x.fail("%s.tagList[%d]: want a non-empty string, got %s", where, i, show(t))
			continue
		}
		if i > 0 && s <= prev {
			x.fail("%s.tagList not sorted/unique: %s", where, show(tl))
		}
		prev = s
	}
	isTimestamp(x, where+".createdAt", m["createdAt"])
	isTimestamp(x, where+".updatedAt", m["updatedAt"])
	if str(m["updatedAt"]) < str(m["createdAt"]) {
		x.fail("%s: updatedAt before createdAt", where)
	}
	isBool(x, where+".favorited", m["favorited"])
	if n, ok := num(m["favoritesCount"]); !ok || n < 0 {
		x.fail("%s.favoritesCount: want a non-negative integer, got %s", where, show(m["favoritesCount"]))
	}
	checkProfile(x, where+".author", m["author"])
}

// checkList validates {"articles": [...], "articlesCount": n} and returns the articles.
func checkList(x *resp) []any {
	m, ok := keys(x, "list", x.v, "articles", "articlesCount")
	if !ok {
		return nil
	}
	list, isArr := m["articles"].([]any)
	if !isArr {
		x.fail("articles: want an array, got %s", show(m["articles"]))
	}
	for i, a := range list {
		checkArticle(x, fmt.Sprintf("articles[%d]", i), a, false)
	}
	for i := 1; i < len(list); i++ {
		if str(at(list[i-1], "createdAt")) < str(at(list[i], "createdAt")) {
			x.fail("articles not ordered by createdAt DESC at %d", i)
		}
	}
	if n, ok := num(m["articlesCount"]); !ok || n < 0 || n > 1000 {
		x.fail("articlesCount: want an integer 0..1000, got %s", show(m["articlesCount"]))
	}
	return list
}

func count(x *resp) int64 {
	n, _ := num(x.at("articlesCount"))
	return n
}

func checkComment(x *resp, where string, v any) {
	m, ok := keys(x, where, v, "id", "createdAt", "updatedAt", "body", "author")
	if !ok {
		return
	}
	if n, ok := num(m["id"]); !ok || n <= 0 {
		x.fail("%s.id: want a positive integer, got %s", where, show(m["id"]))
	}
	isTimestamp(x, where+".createdAt", m["createdAt"])
	isTimestamp(x, where+".updatedAt", m["updatedAt"])
	isString(x, where+".body", m["body"], true)
	checkProfile(x, where+".author", m["author"])
}

func checkComments(x *resp) []any {
	m, ok := keys(x, "comments", x.v, "comments")
	if !ok {
		return nil
	}
	list, isArr := m["comments"].([]any)
	if !isArr {
		x.fail("comments: want an array, got %s", show(m["comments"]))
	}
	for i, c := range list {
		checkComment(x, fmt.Sprintf("comments[%d]", i), c)
	}
	for i := 1; i < len(list); i++ {
		if str(at(list[i-1], "createdAt")) < str(at(list[i], "createdAt")) {
			x.fail("comments not ordered by createdAt DESC at %d", i)
		}
	}
	return list
}

// eq records a failure when got != want (compared as JSON).
func eq(x *resp, what string, got, want any) {
	if show(got) != show(want) {
		x.fail("%s = %s, want %s", what, show(got), show(want))
	}
}
