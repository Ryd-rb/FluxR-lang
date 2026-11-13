package main

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/golang-jwt/jwt/v5"

	"conduitbench/seed"
)

// devArticles: only article ids <= devArticles exist in the 2% dev seed, so the script touches no others.
const devArticles = 10_000

// ---------- tokens ----------

func (r *runner) mint(claims jwt.MapClaims) string {
	s, err := jwt.NewWithClaims(jwt.SigningMethodHS256, claims).SignedString([]byte(r.secret))
	if err != nil {
		panic(err)
	}
	return s
}

// tokenFor mints a valid token for a user id, as the server would.
func (r *runner) tokenFor(id int64) string {
	now := time.Now().Unix()
	return r.mint(jwt.MapClaims{"sub": strconv.FormatInt(id, 10), "iat": now, "exp": now + 30*24*3600})
}

// checkIssuedToken verifies a server-issued token: HS256 with the secret, claims exactly
// {sub: decimal string, iat: int, exp: int} with exp = iat + 30 days. It returns sub.
func (r *runner) checkIssuedToken(x *resp, tok string) string {
	_, err := jwt.Parse(tok, func(*jwt.Token) (any, error) { return []byte(r.secret), nil },
		jwt.WithValidMethods([]string{"HS256"}), jwt.WithExpirationRequired())
	if err != nil {
		x.fail("issued token does not verify: %v", err)
		return ""
	}
	parts := strings.Split(tok, ".")
	payload, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		x.fail("issued token payload: %v", err)
		return ""
	}
	var claims map[string]any
	d := json.NewDecoder(strings.NewReader(string(payload)))
	d.UseNumber()
	if err := d.Decode(&claims); err != nil {
		x.fail("issued token payload: %v", err)
		return ""
	}
	if len(claims) != 3 {
		x.fail("issued token claims %s, want exactly sub, iat, exp", show(claims))
	}
	sub := str(claims["sub"])
	if _, err := strconv.ParseInt(sub, 10, 64); err != nil {
		x.fail("issued token sub %s, want a decimal string", show(claims["sub"]))
	}
	iat, ok1 := num(claims["iat"])
	exp, ok2 := num(claims["exp"])
	if !ok1 || !ok2 || exp-iat != 30*24*3600 {
		x.fail("issued token iat/exp %s/%s, want integers with exp = iat + 30 days", show(claims["iat"]), show(claims["exp"]))
	}
	return sub
}

// ---------- seed-derived fixtures (pure functions of the seed, valid at dev and full scale) ----------

type fixtures struct {
	X             int64 // a seeded article with comments, favorites and 2+ tags
	A             int64 // X's author
	V             int64 // a seeded user who follows authors (id <= 2000: follows exist at 2% scale)
	F             int64 // a seeded user who favorited X
	nComments     int
	nFavorites    int
	tags          []string
	xSlug         string
	userA, userV  string
	userF, emailA string
}

func pickFixtures() fixtures {
	authorArticles := map[int64]int{}
	for id := int64(1); id <= devArticles; id++ {
		authorArticles[seed.ArticleAuthor(id)]++
	}
	var f fixtures
	for id := int64(5000); id <= devArticles; id++ {
		if len(seed.ArticleComments(id)) >= 3 && len(seed.ArticleFavoriters(id)) >= 3 &&
			len(seed.ArticleTags(id)) >= 2 && authorArticles[seed.ArticleAuthor(id)] >= 2 {
			f.X = id
			break
		}
	}
	f.A = seed.ArticleAuthor(f.X)
	for id := int64(100); id <= 2000; id++ {
		if id == f.A {
			continue
		}
		fol := seed.UserFollows(id)
		hits := 0
		for _, a := range fol {
			if authorArticles[a] > 0 {
				hits++
			}
		}
		if len(fol) >= 3 && hits >= 2 {
			f.V = id
			break
		}
	}
	for _, u := range seed.ArticleFavoriters(f.X) {
		if u != f.A && u != f.V {
			f.F = u
			break
		}
	}
	f.nComments = len(seed.ArticleComments(f.X))
	f.nFavorites = len(seed.ArticleFavoriters(f.X))
	for _, t := range seed.ArticleTags(f.X) {
		f.tags = append(f.tags, seed.TagName(t))
	}
	sortStrings(f.tags)
	f.xSlug = seed.ArticleSlug(f.X)
	f.userA, f.userV, f.userF = seed.Username(f.A), seed.Username(f.V), seed.Username(f.F)
	f.emailA = seed.Email(f.A)
	if f.X == 0 || f.V == 0 || f.F == 0 {
		panic("fixture selection failed")
	}
	return f
}

func sortStrings(s []string) {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
}

func hasString(list []any, s string) bool {
	for _, v := range list {
		if str(v) == s {
			return true
		}
	}
	return false
}

func hasID(list []any, id string) bool {
	for _, c := range list {
		if n, ok := num(at(c, "id")); ok && strconv.FormatInt(n, 10) == id {
			return true
		}
	}
	return false
}

func slugs(list []any) []string {
	out := make([]string, len(list))
	for i, a := range list {
		out[i] = str(at(a, "slug"))
	}
	return out
}

func wrap(key string, fields map[string]any) map[string]any { return map[string]any{key: fields} }

// ---------- the script ----------

// pageCheck: page 2 (limit=5&offset=5) must equal items 5..9 of limit=10&offset=0, newest-first by the same order.
func (r *runner) pageCheck(name, base, tok string) {
	x := r.call(name+".page-0-10", "GET", base+"&limit=10&offset=0", tok, nil)
	if !x.expect(200) {
		return
	}
	all := slugs(checkList(x))
	y := r.call(name+".page-5-5", "GET", base+"&limit=5&offset=5", tok, nil)
	if !y.expect(200) {
		return
	}
	want := []string{}
	if len(all) > 5 {
		want = all[5:]
	}
	got := slugs(checkList(y))
	if fmt.Sprint(got) != fmt.Sprint(want) {
		y.fail("offset page = %v, want %v (items 5.. of the first page)", got, want)
	}
}

func (r *runner) script() {
	f := pickFixtures()
	xPath := "/api/articles/" + f.xSlug
	tokV, tokF := r.tokenFor(f.V), r.tokenFor(f.F)
	const missingSlug = "no-such-article-999999999"

	// ===== tags =====
	x := r.call("tags.list", "GET", "/api/tags", "", nil)
	var nTags int
	if x.expect(200) {
		if _, ok := keys(x, "tags", x.v, "tags"); ok {
			tags := arr(x.at("tags"))
			nTags = len(tags)
			if nTags < seed.NTags {
				x.fail("%d tags, want at least %d", nTags, seed.NTags)
			}
			for i, t := range tags {
				isString(x, "tags[]", t, true)
				if i > 0 && str(t) <= str(tags[i-1]) {
					x.fail("tags not sorted/unique at %d", i)
				}
			}
		}
	}
	x = r.call("tags.list.ignores-auth-header", "GET", "/api/tags", "raw:Token not-a-jwt", nil)
	x.expect(200)

	// ===== register =====
	const (
		uname  = "conformance_user"
		email  = "conformance_user@example.com"
		pass   = "conformance-pass-1"
		uname2 = "conformance_user2"
		email2 = "conformance_user2@example.com"
		pass2  = "conformance-pass-2"
	)
	x = r.call("users.register", "POST", "/api/users", "", wrap("user", map[string]any{"username": uname, "email": email, "password": pass}))
	var tokReg, newID string
	if x.expect(201) {
		checkUser(x, x.at("user"))
		eq(x, "user.username", x.at("user", "username"), uname)
		eq(x, "user.email", x.at("user", "email"), email)
		eq(x, "user.bio", x.at("user", "bio"), nil)
		eq(x, "user.image", x.at("user", "image"), nil)
		tokReg = str(x.at("user", "token"))
		newID = r.checkIssuedToken(x, tokReg)
	}
	if tokReg == "" {
		panic(abort("register failed; cannot continue"))
	}
	r.call("users.register.duplicate-username", "POST", "/api/users", "", wrap("user", map[string]any{"username": uname, "email": "other_" + email, "password": pass})).expect(422)
	r.call("users.register.duplicate-email", "POST", "/api/users", "", wrap("user", map[string]any{"username": "other_" + uname, "email": email, "password": pass})).expect(422)
	r.call("users.register.duplicate-seeded-username", "POST", "/api/users", "", wrap("user", map[string]any{"username": f.userA, "email": "x_" + email, "password": pass})).expect(422)
	r.call("users.register.missing-password", "POST", "/api/users", "", wrap("user", map[string]any{"username": "u_" + uname, "email": "u_" + email})).expect(422)
	r.call("users.register.empty-username", "POST", "/api/users", "", wrap("user", map[string]any{"username": "", "email": "u_" + email, "password": pass})).expect(422)
	r.call("users.register.null-email", "POST", "/api/users", "", wrap("user", map[string]any{"username": "u_" + uname, "email": nil, "password": pass})).expect(422)
	r.call("users.register.number-username", "POST", "/api/users", "", wrap("user", map[string]any{"username": 123, "email": "u_" + email, "password": pass})).expect(422)
	r.call("users.register.no-wrapper", "POST", "/api/users", "", map[string]any{"username": "u_" + uname, "email": "u_" + email, "password": pass}).expect(422)
	r.call("users.register.wrapper-not-object", "POST", "/api/users", "", map[string]any{"user": "u_" + uname}).expect(422)
	r.call("users.register.malformed-json", "POST", "/api/users", "", `{"user": {"username": "u_x",`).expect(422)
	r.call("users.register.empty-body", "POST", "/api/users", "", "").expect(422)
	r.call("users.register.password-over-72-bytes", "POST", "/api/users", "", wrap("user", map[string]any{"username": "u_" + uname, "email": "u_" + email, "password": strings.Repeat("p", 73)})).expect(422)
	x = r.call("users.register.extra-fields-ignored", "POST", "/api/users", "", map[string]any{"extra": 1, "user": map[string]any{"username": "conformance_extra", "email": "conformance_extra@example.com", "password": pass, "bio": "ignored", "admin": true}})
	if x.expect(201) {
		checkUser(x, x.at("user"))
		eq(x, "user.bio", x.at("user", "bio"), nil)
	}

	// ===== login =====
	x = r.call("users.login", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": email, "password": pass}))
	var tokNew string
	if x.expect(200) {
		checkUser(x, x.at("user"))
		eq(x, "user.username", x.at("user", "username"), uname)
		tokNew = str(x.at("user", "token"))
		if sub := r.checkIssuedToken(x, tokNew); sub != newID {
			x.fail("login token sub %q, register token sub %q", sub, newID)
		}
	}
	if tokNew == "" {
		panic(abort("login failed; cannot continue"))
	}
	r.call("users.login.wrong-password", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": email, "password": "wrong-password"})).expect(401)
	r.call("users.login.unknown-email", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": "nobody@example.com", "password": pass})).expect(401)
	r.call("users.login.missing-password", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": email})).expect(422)
	r.call("users.login.empty-email", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": "", "password": pass})).expect(422)
	r.call("users.login.no-wrapper", "POST", "/api/users/login", "", map[string]any{"email": email, "password": pass}).expect(422)

	x = r.call("users.login.seeded-author", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": f.emailA, "password": seed.Password}))
	var tokA string
	if x.expect(200) {
		checkUser(x, x.at("user"))
		bio, image := seed.UserBioImage(f.A)
		eq(x, "user.username", x.at("user", "username"), f.userA)
		eq(x, "user.email", x.at("user", "email"), f.emailA)
		eq(x, "user.bio", x.at("user", "bio"), bio)
		eq(x, "user.image", x.at("user", "image"), image)
		tokA = str(x.at("user", "token"))
		if sub := r.checkIssuedToken(x, tokA); sub != strconv.FormatInt(f.A, 10) {
			x.fail("token sub %q, want %d", sub, f.A)
		}
	}
	if tokA == "" {
		panic(abort("seeded login failed; cannot continue"))
	}

	// ===== current user and token validation =====
	x = r.call("user.get", "GET", "/api/user", tokNew, nil)
	if x.expect(200) {
		checkUser(x, x.at("user"))
		eq(x, "user.username", x.at("user", "username"), uname)
		if str(x.at("user", "token")) != tokNew {
			x.fail("user.token is not the presented token")
		}
	}
	x = r.call("user.get.register-token", "GET", "/api/user", tokReg, nil)
	if x.expect(200) && str(x.at("user", "token")) != tokReg {
		x.fail("user.token is not the presented token")
	}
	x = r.call("user.get.minted-token-seeded", "GET", "/api/user", tokV, nil)
	if x.expect(200) {
		checkUser(x, x.at("user"))
		bio, image := seed.UserBioImage(f.V)
		eq(x, "user.username", x.at("user", "username"), f.userV)
		eq(x, "user.email", x.at("user", "email"), seed.Email(f.V))
		eq(x, "user.bio", x.at("user", "bio"), bio)
		eq(x, "user.image", x.at("user", "image"), image)
	}
	now := time.Now().Unix()
	vSub := strconv.FormatInt(f.V, 10)
	badTokens := []struct{ name, tok string }{
		{"no-header", ""},
		{"bearer-scheme", "raw:Bearer " + tokV},
		{"no-scheme", "raw:" + tokV},
		{"garbage", "not.a.jwt"},
		{"wrong-secret", func() string {
			s, _ := jwt.NewWithClaims(jwt.SigningMethodHS256, jwt.MapClaims{"sub": vSub, "iat": now, "exp": now + 3600}).SignedString([]byte(r.secret + "-wrong"))
			return s
		}()},
		{"expired", r.mint(jwt.MapClaims{"sub": vSub, "iat": now - 40*24*3600, "exp": now - 10*24*3600})},
		{"no-exp", r.mint(jwt.MapClaims{"sub": vSub, "iat": now})},
		{"numeric-sub", r.mint(jwt.MapClaims{"sub": f.V, "iat": now, "exp": now + 3600})},
		{"non-numeric-sub", r.mint(jwt.MapClaims{"sub": "abc", "iat": now, "exp": now + 3600})},
		{"alg-none", func() string {
			s, _ := jwt.NewWithClaims(jwt.SigningMethodNone, jwt.MapClaims{"sub": vSub, "iat": now, "exp": now + 3600}).SignedString(jwt.UnsafeAllowNoneSignatureType)
			return s
		}()},
		{"unknown-user", r.tokenFor(999_999_999)},
	}
	for _, b := range badTokens {
		r.call("user.get.401."+b.name, "GET", "/api/user", b.tok, nil).expect(401)
	}

	// ===== update user =====
	x = r.call("user.update.bio-image", "PUT", "/api/user", tokNew, wrap("user", map[string]any{"bio": "Conformance bio", "image": "https://example.com/c.png"}))
	if x.expect(200) {
		checkUser(x, x.at("user"))
		eq(x, "user.bio", x.at("user", "bio"), "Conformance bio")
		eq(x, "user.image", x.at("user", "image"), "https://example.com/c.png")
		eq(x, "user.username", x.at("user", "username"), uname)
		eq(x, "user.email", x.at("user", "email"), email)
		if str(x.at("user", "token")) != tokNew {
			x.fail("user.token is not the presented token")
		}
	}
	x = r.call("user.update.nulls-unchanged", "PUT", "/api/user", tokNew, wrap("user", map[string]any{"bio": nil, "email": nil, "image": nil, "username": nil, "password": nil}))
	if x.expect(200) {
		eq(x, "user.bio", x.at("user", "bio"), "Conformance bio")
		eq(x, "user.email", x.at("user", "email"), email)
	}
	x = r.call("user.update.empty-object", "PUT", "/api/user", tokNew, wrap("user", map[string]any{}))
	if x.expect(200) {
		eq(x, "user.image", x.at("user", "image"), "https://example.com/c.png")
	}
	r.call("user.update.taken-username", "PUT", "/api/user", tokNew, wrap("user", map[string]any{"username": f.userV})).expect(422)
	r.call("user.update.taken-email", "PUT", "/api/user", tokNew, wrap("user", map[string]any{"email": seed.Email(f.V)})).expect(422)
	r.call("user.update.empty-email", "PUT", "/api/user", tokNew, wrap("user", map[string]any{"email": ""})).expect(422)
	r.call("user.update.number-bio", "PUT", "/api/user", tokNew, wrap("user", map[string]any{"bio": 5})).expect(422)
	r.call("user.update.no-wrapper", "PUT", "/api/user", tokNew, map[string]any{"bio": "x"}).expect(422)
	r.call("user.update.no-auth", "PUT", "/api/user", "", wrap("user", map[string]any{"bio": "x"})).expect(401)
	x = r.call("user.update.empty-bio", "PUT", "/api/user", tokNew, wrap("user", map[string]any{"bio": ""}))
	if x.expect(200) {
		eq(x, "user.bio", x.at("user", "bio"), "")
	}
	x = r.call("user.update.username-email-password", "PUT", "/api/user", tokNew, wrap("user", map[string]any{"username": uname2, "email": email2, "password": pass2, "bio": "Conformance bio 2"}))
	if x.expect(200) {
		eq(x, "user.username", x.at("user", "username"), uname2)
		eq(x, "user.email", x.at("user", "email"), email2)
		eq(x, "user.bio", x.at("user", "bio"), "Conformance bio 2")
	}
	r.call("users.login.old-password", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": email2, "password": pass})).expect(401)
	r.call("users.login.old-email", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": email, "password": pass2})).expect(401)
	x = r.call("users.login.new-password", "POST", "/api/users/login", "", wrap("user", map[string]any{"email": email2, "password": pass2}))
	if x.expect(200) {
		eq(x, "user.username", x.at("user", "username"), uname2)
	}
	x = r.call("user.get.after-update", "GET", "/api/user", tokNew, nil)
	if x.expect(200) {
		eq(x, "user.username", x.at("user", "username"), uname2)
	}

	// ===== profiles and follows =====
	x = r.call("profile.get.anonymous", "GET", "/api/profiles/"+f.userA, "", nil)
	if x.expect(200) {
		keys(x, "body", x.v, "profile")
		checkProfile(x, "profile", x.at("profile"))
		bio, image := seed.UserBioImage(f.A)
		eq(x, "profile.username", x.at("profile", "username"), f.userA)
		eq(x, "profile.bio", x.at("profile", "bio"), bio)
		eq(x, "profile.image", x.at("profile", "image"), image)
		eq(x, "profile.following", x.at("profile", "following"), false)
	}
	r.call("profile.get.unknown", "GET", "/api/profiles/no_such_user_zz", "", nil).expect(404)
	r.call("profile.get.invalid-token", "GET", "/api/profiles/"+f.userA, "garbage", nil).expect(401)
	x = r.call("profile.get.self-updated", "GET", "/api/profiles/"+uname2, "", nil)
	if x.expect(200) {
		eq(x, "profile.bio", x.at("profile", "bio"), "Conformance bio 2")
	}
	r.call("profile.get.old-username", "GET", "/api/profiles/"+uname, "", nil).expect(404)

	// The new user's feed starts empty.
	x = r.call("feed.empty", "GET", "/api/articles/feed", tokNew, nil)
	if x.expect(200) {
		if len(checkList(x)) != 0 || count(x) != 0 {
			x.fail("feed of a user who follows nobody must be empty with articlesCount 0")
		}
	}
	r.call("profile.follow.no-auth", "POST", "/api/profiles/"+f.userA+"/follow", "", nil).expect(401)
	r.call("profile.follow.unknown", "POST", "/api/profiles/no_such_user_zz/follow", tokNew, nil).expect(404)
	x = r.call("profile.follow", "POST", "/api/profiles/"+f.userA+"/follow", tokNew, nil)
	if x.expect(200) {
		checkProfile(x, "profile", x.at("profile"))
		eq(x, "profile.following", x.at("profile", "following"), true)
		eq(x, "profile.username", x.at("profile", "username"), f.userA)
	}
	x = r.call("profile.follow.again", "POST", "/api/profiles/"+f.userA+"/follow", tokNew, nil)
	if x.expect(200) {
		eq(x, "profile.following", x.at("profile", "following"), true)
	}
	x = r.call("profile.get.following", "GET", "/api/profiles/"+f.userA, tokNew, nil)
	if x.expect(200) {
		eq(x, "profile.following", x.at("profile", "following"), true)
	}
	x = r.call("profile.get.other-viewer", "GET", "/api/profiles/"+f.userA, tokF, nil)
	if x.expect(200) {
		checkProfile(x, "profile", x.at("profile"))
	}

	// Feed after following A: exactly A's articles, newest first.
	authorPath := "/api/articles?author=" + f.userA + "&limit=10"
	xa := r.call("articles.list.author-as-follower", "GET", authorPath, tokNew, nil)
	var aSlugs []string
	var aCount int64
	if xa.expect(200) {
		list := checkList(xa)
		aSlugs, aCount = slugs(list), count(xa)
		if len(list) < 2 {
			xa.fail("author %s should have at least 2 articles, got %d", f.userA, len(list))
		}
		for i, a := range list {
			eq(xa, fmt.Sprintf("articles[%d].author.username", i), at(a, "author", "username"), f.userA)
			eq(xa, fmt.Sprintf("articles[%d].author.following", i), at(a, "author", "following"), true)
		}
	}
	x = r.call("feed.after-follow", "GET", "/api/articles/feed?limit=10", tokNew, nil)
	if x.expect(200) {
		list := checkList(x)
		eq(x, "feed slugs", slugs(list), aSlugs)
		eq(x, "feed articlesCount", count(x), aCount)
		for i, a := range list {
			eq(x, fmt.Sprintf("articles[%d].author.following", i), at(a, "author", "following"), true)
		}
	}
	x = r.call("feed.offset", "GET", "/api/articles/feed?limit=1&offset=1", tokNew, nil)
	if x.expect(200) && len(aSlugs) > 1 {
		eq(x, "feed page 2 slugs", slugs(checkList(x)), aSlugs[1:2])
	}
	r.call("feed.no-auth", "GET", "/api/articles/feed", "", nil).expect(401)
	r.call("feed.invalid-token", "GET", "/api/articles/feed", "garbage", nil).expect(401)
	r.call("feed.limit-0", "GET", "/api/articles/feed?limit=0", tokNew, nil).expect(422)
	x = r.call("profile.unfollow", "DELETE", "/api/profiles/"+f.userA+"/follow", tokNew, nil)
	if x.expect(200) {
		checkProfile(x, "profile", x.at("profile"))
		eq(x, "profile.following", x.at("profile", "following"), false)
	}
	x = r.call("profile.unfollow.again", "DELETE", "/api/profiles/"+f.userA+"/follow", tokNew, nil)
	if x.expect(200) {
		eq(x, "profile.following", x.at("profile", "following"), false)
	}
	r.call("profile.unfollow.unknown", "DELETE", "/api/profiles/no_such_user_zz/follow", tokNew, nil).expect(404)
	r.call("profile.unfollow.no-auth", "DELETE", "/api/profiles/"+f.userA+"/follow", "", nil).expect(401)
	x = r.call("profile.get.unfollowed", "GET", "/api/profiles/"+f.userA, tokNew, nil)
	if x.expect(200) {
		eq(x, "profile.following", x.at("profile", "following"), false)
	}
	x = r.call("feed.after-unfollow", "GET", "/api/articles/feed", tokNew, nil)
	if x.expect(200) {
		if len(checkList(x)) != 0 || count(x) != 0 {
			x.fail("feed must be empty after unfollowing")
		}
	}
	x = r.call("feed.seeded-user", "GET", "/api/articles/feed?limit=5", tokV, nil)
	if x.expect(200) {
		list := checkList(x)
		if len(list) == 0 {
			x.fail("seeded user %d follows authors; feed must not be empty", f.V)
		}
		for i, a := range list {
			eq(x, fmt.Sprintf("articles[%d].author.following", i), at(a, "author", "following"), true)
		}
	}

	// ===== article list =====
	x = r.call("articles.list.default", "GET", "/api/articles", "", nil)
	var top10 []string
	if x.expect(200) {
		list := checkList(x)
		if len(list) != 20 {
			x.fail("default limit: got %d articles, want 20", len(list))
		}
		if count(x) != 1000 {
			x.fail("articlesCount %d, want 1000 (capped)", count(x))
		}
		for i, a := range list {
			eq(x, fmt.Sprintf("articles[%d].favorited", i), at(a, "favorited"), false)
			eq(x, fmt.Sprintf("articles[%d].author.following", i), at(a, "author", "following"), false)
		}
		top10 = slugs(list)
		if len(top10) > 10 {
			top10 = top10[:10]
		}
	}
	x = r.call("articles.list.limit-1", "GET", "/api/articles?limit=1", "", nil)
	if x.expect(200) {
		eq(x, "slugs", slugs(checkList(x)), top10[:min(1, len(top10))])
	}
	x = r.call("articles.list.limit-100", "GET", "/api/articles?limit=100", "", nil)
	if x.expect(200) {
		if n := len(checkList(x)); n != 100 {
			x.fail("limit=100: got %d articles", n)
		}
	}
	x = r.call("articles.list.offset-5", "GET", "/api/articles?limit=5&offset=5", "", nil)
	if x.expect(200) && len(top10) == 10 {
		eq(x, "slugs", slugs(checkList(x)), top10[5:10])
	}
	x = r.call("articles.list.leading-zeros", "GET", "/api/articles?limit=05&offset=005", "", nil)
	if x.expect(200) && len(top10) == 10 {
		eq(x, "slugs", slugs(checkList(x)), top10[5:10])
	}
	x = r.call("articles.list.offset-past-end", "GET", "/api/articles?offset=100000000", "", nil)
	if x.expect(200) {
		if len(checkList(x)) != 0 {
			x.fail("want an empty page")
		}
		if count(x) != 1000 {
			x.fail("articlesCount %d, want 1000", count(x))
		}
	}
	for _, bad := range []struct{ name, qs string }{
		{"limit-0", "limit=0"}, {"limit-101", "limit=101"}, {"limit-negative", "limit=-1"},
		{"limit-abc", "limit=abc"}, {"limit-empty", "limit="}, {"limit-decimal", "limit=1.5"},
		{"limit-plus-sign", "limit=%2B5"}, {"limit-space", "limit=%205"},
		{"offset-negative", "offset=-1"}, {"offset-abc", "offset=abc"}, {"offset-empty", "offset="},
		{"offset-overflow", "offset=99999999999999999999"},
	} {
		r.call("articles.list.422."+bad.name, "GET", "/api/articles?"+bad.qs, "", nil).expect(422)
	}
	r.call("articles.list.invalid-token", "GET", "/api/articles", "garbage", nil).expect(401)

	tag := f.tags[0]
	x = r.call("articles.list.tag", "GET", "/api/articles?tag="+tag+"&limit=5", "", nil)
	var tagSlugs []string
	var tagCount int64
	if x.expect(200) {
		list := checkList(x)
		tagSlugs, tagCount = slugs(list), count(x)
		if len(list) != 5 {
			x.fail("tag %s: got %d articles, want 5", tag, len(list))
		}
		for i, a := range list {
			if !hasString(arr(at(a, "tagList")), tag) {
				x.fail("articles[%d].tagList %s lacks %s", i, show(at(a, "tagList")), tag)
			}
		}
	}
	r.pageCheck("articles.list.tag", "/api/articles?tag="+tag, "")
	r.pageCheck("articles.list.author", "/api/articles?author="+f.userA, "")
	r.pageCheck("articles.list.favorited", "/api/articles?favorited="+f.userF, tokF)
	r.pageCheck("feed", "/api/articles/feed?", tokV)
	x = r.call("articles.list.tag-beats-author", "GET", "/api/articles?author="+f.userV+"&tag="+tag+"&limit=5", "", nil)
	if x.expect(200) {
		eq(x, "slugs", slugs(checkList(x)), tagSlugs)
		eq(x, "articlesCount", count(x), tagCount)
	}
	x = r.call("articles.list.empty-tag-ignored", "GET", "/api/articles?tag=&author="+f.userA+"&limit=10", "", nil)
	if x.expect(200) {
		eq(x, "slugs", slugs(checkList(x)), aSlugs)
	}
	x = r.call("articles.list.author", "GET", "/api/articles?author="+f.userA+"&limit=10", "", nil)
	if x.expect(200) {
		list := checkList(x)
		eq(x, "slugs", slugs(list), aSlugs)
		eq(x, "articlesCount", count(x), aCount)
		for i, a := range list {
			eq(x, fmt.Sprintf("articles[%d].author.following", i), at(a, "author", "following"), false)
		}
	}
	x = r.call("articles.list.author-beats-favorited", "GET", "/api/articles?favorited="+f.userF+"&author="+f.userA+"&limit=10", "", nil)
	if x.expect(200) {
		eq(x, "slugs", slugs(checkList(x)), aSlugs)
	}
	favPath := "/api/articles?favorited=" + f.userF + "&limit=5"
	x = r.call("articles.list.favorited-as-favoriter", "GET", favPath, tokF, nil)
	if x.expect(200) {
		list := checkList(x)
		if len(list) == 0 {
			x.fail("user %s favorited %s; list must not be empty", f.userF, f.xSlug)
		}
		for i, a := range list {
			eq(x, fmt.Sprintf("articles[%d].favorited", i), at(a, "favorited"), true)
		}
	}
	x = r.call("articles.list.favorited-anonymous", "GET", favPath, "", nil)
	if x.expect(200) {
		for i, a := range checkList(x) {
			eq(x, fmt.Sprintf("articles[%d].favorited", i), at(a, "favorited"), false)
		}
	}
	x = r.call("articles.list.favorited-other-viewer", "GET", favPath, tokV, nil)
	if x.expect(200) {
		checkList(x)
	}
	for _, u := range []struct{ name, qs string }{
		{"unknown-tag", "tag=no_such_tag_zz"}, {"unknown-author", "author=no_such_user_zz"}, {"unknown-favorited", "favorited=no_such_user_zz"},
	} {
		x = r.call("articles.list."+u.name, "GET", "/api/articles?"+u.qs, "", nil)
		if x.expect(200) {
			if len(checkList(x)) != 0 || count(x) != 0 {
				x.fail("want an empty list with articlesCount 0")
			}
		}
	}

	// ===== single article (seeded) =====
	x = r.call("article.get.seeded", "GET", xPath, "", nil)
	var favs0 int64 = -1
	if x.expect(200) {
		keys(x, "body", x.v, "article")
		checkArticle(x, "article", x.at("article"), true)
		desc, body := seed.ArticleText(f.X)
		created := seed.ArticleCreated(f.X).UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")
		eq(x, "article.slug", x.at("article", "slug"), f.xSlug)
		eq(x, "article.title", x.at("article", "title"), seed.ArticleTitle(f.X))
		eq(x, "article.description", x.at("article", "description"), desc)
		eq(x, "article.body", x.at("article", "body"), body)
		eq(x, "article.tagList", x.at("article", "tagList"), f.tags)
		eq(x, "article.createdAt", x.at("article", "createdAt"), created)
		eq(x, "article.updatedAt", x.at("article", "updatedAt"), created)
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), f.nFavorites)
		eq(x, "article.favorited", x.at("article", "favorited"), false)
		eq(x, "article.author.username", x.at("article", "author", "username"), f.userA)
		favs0, _ = num(x.at("article", "favoritesCount"))
	}
	x = r.call("article.get.as-favoriter", "GET", xPath, tokF, nil)
	if x.expect(200) {
		eq(x, "article.favorited", x.at("article", "favorited"), true)
	}
	r.call("article.get.unknown", "GET", "/api/articles/"+missingSlug, "", nil).expect(404)
	r.call("article.get.invalid-token", "GET", xPath, "garbage", nil).expect(401)

	// ===== favorites =====
	r.call("favorite.no-auth", "POST", xPath+"/favorite", "", nil).expect(401)
	r.call("favorite.unknown", "POST", "/api/articles/"+missingSlug+"/favorite", tokNew, nil).expect(404)
	x = r.call("favorite", "POST", xPath+"/favorite", tokNew, nil)
	if x.expect(200) {
		checkArticle(x, "article", x.at("article"), true)
		eq(x, "article.favorited", x.at("article", "favorited"), true)
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), favs0+1)
	}
	x = r.call("favorite.again", "POST", xPath+"/favorite", tokNew, nil)
	if x.expect(200) {
		eq(x, "article.favorited", x.at("article", "favorited"), true)
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), favs0+1)
	}
	x = r.call("article.get.favorited", "GET", xPath, tokNew, nil)
	if x.expect(200) {
		eq(x, "article.favorited", x.at("article", "favorited"), true)
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), favs0+1)
	}
	x = r.call("articles.list.favorited-by-new-user", "GET", "/api/articles?favorited="+uname2, tokNew, nil)
	if x.expect(200) {
		eq(x, "slugs", slugs(checkList(x)), []string{f.xSlug})
		eq(x, "articlesCount", count(x), 1)
		if l := arr(x.at("articles")); len(l) == 1 {
			eq(x, "articles[0].favorited", at(l[0], "favorited"), true)
		}
	}
	x = r.call("unfavorite", "DELETE", xPath+"/favorite", tokNew, nil)
	if x.expect(200) {
		checkArticle(x, "article", x.at("article"), true)
		eq(x, "article.favorited", x.at("article", "favorited"), false)
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), favs0)
	}
	x = r.call("unfavorite.again", "DELETE", xPath+"/favorite", tokNew, nil)
	if x.expect(200) {
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), favs0)
	}
	r.call("unfavorite.unknown", "DELETE", "/api/articles/"+missingSlug+"/favorite", tokNew, nil).expect(404)
	r.call("unfavorite.no-auth", "DELETE", xPath+"/favorite", "", nil).expect(401)

	// ===== comments =====
	x = r.call("comments.list.seeded", "GET", xPath+"/comments", "", nil)
	var seededComment string
	if x.expect(200) {
		list := checkComments(x)
		if len(list) != f.nComments {
			x.fail("%d comments, want %d", len(list), f.nComments)
		}
		for _, c := range list {
			if str(at(c, "author", "username")) != uname2 {
				n, _ := num(at(c, "id"))
				seededComment = strconv.FormatInt(n, 10)
				break
			}
		}
	}
	x = r.call("comments.list.as-viewer", "GET", xPath+"/comments", tokV, nil)
	if x.expect(200) {
		checkComments(x)
	}
	r.call("comments.list.unknown", "GET", "/api/articles/"+missingSlug+"/comments", "", nil).expect(404)
	r.call("comments.list.invalid-token", "GET", xPath+"/comments", "garbage", nil).expect(401)
	r.call("comments.create.no-auth", "POST", xPath+"/comments", "", wrap("comment", map[string]any{"body": "x"})).expect(401)
	r.call("comments.create.empty-body", "POST", xPath+"/comments", tokNew, wrap("comment", map[string]any{"body": ""})).expect(422)
	r.call("comments.create.missing-body", "POST", xPath+"/comments", tokNew, wrap("comment", map[string]any{})).expect(422)
	r.call("comments.create.no-wrapper", "POST", xPath+"/comments", tokNew, map[string]any{"body": "x"}).expect(422)
	r.call("comments.create.unknown-article", "POST", "/api/articles/"+missingSlug+"/comments", tokNew, wrap("comment", map[string]any{"body": "x"})).expect(404)
	r.call("comments.create.unknown-article-invalid-body", "POST", "/api/articles/"+missingSlug+"/comments", tokNew, wrap("comment", map[string]any{"body": ""})).expect(422)
	x = r.call("comments.create", "POST", xPath+"/comments", tokNew, wrap("comment", map[string]any{"body": "Conformance comment <b>&</b> \"quoted\" ünïcode"}))
	var cid string
	if x.expect(201) {
		keys(x, "body", x.v, "comment")
		checkComment(x, "comment", x.at("comment"))
		eq(x, "comment.body", x.at("comment", "body"), "Conformance comment <b>&</b> \"quoted\" ünïcode")
		eq(x, "comment.author.username", x.at("comment", "author", "username"), uname2)
		eq(x, "comment.author.bio", x.at("comment", "author", "bio"), "Conformance bio 2")
		eq(x, "comment.author.following", x.at("comment", "author", "following"), false)
		eq(x, "comment.updatedAt", x.at("comment", "updatedAt"), x.at("comment", "createdAt"))
		n, _ := num(x.at("comment", "id"))
		cid = strconv.FormatInt(n, 10)
		r.commentIDs[cid] = true
	}
	if cid == "" {
		panic(abort("comment create failed; cannot continue"))
	}
	x = r.call("comments.list.after-create", "GET", xPath+"/comments", tokNew, nil)
	if x.expect(200) {
		list := checkComments(x)
		if len(list) != f.nComments+1 {
			x.fail("%d comments, want %d", len(list), f.nComments+1)
		} else {
			n, _ := num(at(list[0], "id"))
			if strconv.FormatInt(n, 10) != cid {
				x.fail("the new comment must be first (newest)")
			}
		}
	}
	// Following the commenter shows up in the comment's author.following.
	r.call("profile.follow.commenter", "POST", "/api/profiles/"+uname2+"/follow", tokV, nil).expect(200)
	x = r.call("comments.list.follower-viewer", "GET", xPath+"/comments", tokV, nil)
	if x.expect(200) {
		if list := checkComments(x); len(list) > 0 {
			eq(x, "comments[0].author.following", at(list[0], "author", "following"), true)
		}
	}
	r.call("profile.unfollow.commenter", "DELETE", "/api/profiles/"+uname2+"/follow", tokV, nil).expect(200)
	cPath := xPath + "/comments/" + cid
	r.call("comments.delete.no-auth", "DELETE", cPath, "", nil).expect(401)
	r.call("comments.delete.not-author", "DELETE", cPath, tokA, nil).expect(403)
	r.call("comments.delete.wrong-article", "DELETE", "/api/articles/"+seed.ArticleSlug(f.X-1)+"/comments/"+cid, tokNew, nil).expect(404)
	r.call("comments.delete.unknown-article", "DELETE", "/api/articles/"+missingSlug+"/comments/"+cid, tokNew, nil).expect(404)
	r.call("comments.delete.non-integer-id", "DELETE", xPath+"/comments/abc", tokNew, nil).expect(404)
	r.call("comments.delete.unknown-id", "DELETE", xPath+"/comments/999999999", tokNew, nil).expect(404)
	if seededComment != "" {
		r.call("comments.delete.seeded-not-author", "DELETE", xPath+"/comments/"+seededComment, tokNew, nil).expect(403)
	}
	r.call("comments.delete", "DELETE", cPath, tokNew, nil).expect(204)
	x = r.call("comments.list.after-delete", "GET", xPath+"/comments", "", nil)
	if x.expect(200) {
		list := checkComments(x)
		if len(list) != f.nComments {
			x.fail("%d comments, want %d", len(list), f.nComments)
		}
		if hasID(list, cid) {
			x.fail("deleted comment still listed")
		}
	}
	r.call("comments.delete.again", "DELETE", cPath, tokNew, nil).expect(404)

	// ===== create / update / delete article =====
	const newTag = "zzconformancetag"
	title := "Conformance: Hello, World! 2026"
	art := map[string]any{"title": title, "description": "Conformance description", "body": "Conformance body\n\nSecond paragraph.", "tagList": []string{newTag, tag, newTag}}
	r.call("articles.create.no-auth", "POST", "/api/articles", "", wrap("article", art)).expect(401)
	for _, bad := range []struct {
		name string
		body any
	}{
		{"missing-title", wrap("article", map[string]any{"description": "d", "body": "b"})},
		{"empty-description", wrap("article", map[string]any{"title": "t", "description": "", "body": "b"})},
		{"null-body", wrap("article", map[string]any{"title": "t", "description": "d", "body": nil})},
		{"number-title", wrap("article", map[string]any{"title": 7, "description": "d", "body": "b"})},
		{"taglist-not-array", wrap("article", map[string]any{"title": "t", "description": "d", "body": "b", "tagList": "x"})},
		{"taglist-empty-string", wrap("article", map[string]any{"title": "t", "description": "d", "body": "b", "tagList": []any{"ok", ""}})},
		{"taglist-number", wrap("article", map[string]any{"title": "t", "description": "d", "body": "b", "tagList": []any{1}})},
		{"taglist-null-item", wrap("article", map[string]any{"title": "t", "description": "d", "body": "b", "tagList": []any{nil}})},
		{"no-wrapper", map[string]any{"title": "t", "description": "d", "body": "b"}},
		{"malformed-json", `{"article": `},
	} {
		r.call("articles.create.422."+bad.name, "POST", "/api/articles", tokNew, bad.body).expect(422)
	}
	slugRe := regexp.MustCompile(`^conformance-hello-world-2026-(\d+)$`)
	x = r.call("articles.create", "POST", "/api/articles", tokNew, wrap("article", art))
	var newSlug string
	if x.expect(201) {
		keys(x, "body", x.v, "article")
		checkArticle(x, "article", x.at("article"), true)
		newSlug = str(x.at("article", "slug"))
		if !slugRe.MatchString(newSlug) {
			x.fail("slug %q, want conformance-hello-world-2026-<id>", newSlug)
		}
		eq(x, "article.title", x.at("article", "title"), title)
		eq(x, "article.description", x.at("article", "description"), "Conformance description")
		eq(x, "article.body", x.at("article", "body"), "Conformance body\n\nSecond paragraph.")
		want := []string{tag, newTag}
		sortStrings(want)
		eq(x, "article.tagList", x.at("article", "tagList"), want)
		eq(x, "article.favorited", x.at("article", "favorited"), false)
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), 0)
		eq(x, "article.updatedAt", x.at("article", "updatedAt"), x.at("article", "createdAt"))
		eq(x, "article.author.username", x.at("article", "author", "username"), uname2)
		eq(x, "article.author.following", x.at("article", "author", "following"), false)
	}
	if newSlug == "" {
		panic(abort("article create failed; cannot continue"))
	}
	newPath := "/api/articles/" + newSlug
	for i, tl := range []struct {
		name string
		v    any
	}{{"taglist-empty", []string{}}, {"taglist-null", nil}, {"taglist-absent", "absent"}} {
		body := map[string]any{"title": fmt.Sprintf("Conformance no tags %d", i+1), "description": "d", "body": "b"}
		if tl.v != "absent" {
			body["tagList"] = tl.v
		}
		x = r.call("articles.create."+tl.name, "POST", "/api/articles", tokNew, wrap("article", body))
		if x.expect(201) {
			checkArticle(x, "article", x.at("article"), true)
			eq(x, "article.tagList", x.at("article", "tagList"), []string{})
		}
	}
	x = r.call("tags.list.after-create", "GET", "/api/tags", "", nil)
	if x.expect(200) {
		tags := arr(x.at("tags"))
		if len(tags) != nTags+1 {
			x.fail("%d tags, want %d (one new tag)", len(tags), nTags+1)
		}
		if !hasString(tags, newTag) {
			x.fail("new tag %s missing", newTag)
		}
		for i := 1; i < len(tags); i++ {
			if str(tags[i]) <= str(tags[i-1]) {
				x.fail("tags not sorted/unique at %d", i)
			}
		}
	}
	x = r.call("article.get.created", "GET", newPath, "", nil)
	if x.expect(200) {
		checkArticle(x, "article", x.at("article"), true)
		eq(x, "article.slug", x.at("article", "slug"), newSlug)
	}
	x = r.call("articles.list.newest-first", "GET", "/api/articles?limit=4", "", nil)
	if x.expect(200) {
		s := slugs(checkList(x))
		if len(s) != 4 || s[3] != newSlug {
			x.fail("the four articles created by the run must be the newest, oldest of them %s; got %v", newSlug, s)
		}
	}
	x = r.call("articles.list.new-tag", "GET", "/api/articles?tag="+newTag, "", nil)
	if x.expect(200) {
		eq(x, "slugs", slugs(checkList(x)), []string{newSlug})
		eq(x, "articlesCount", count(x), 1)
	}
	x = r.call("articles.list.new-author", "GET", "/api/articles?author="+uname2, "", nil)
	if x.expect(200) {
		if l := checkList(x); len(l) != 4 || count(x) != 4 {
			x.fail("want the 4 articles the new user created")
		}
	}

	r.call("articles.update.no-auth", "PUT", newPath, "", wrap("article", map[string]any{"body": "x"})).expect(401)
	r.call("articles.update.not-author", "PUT", newPath, tokA, wrap("article", map[string]any{"body": "x"})).expect(403)
	r.call("articles.update.unknown", "PUT", "/api/articles/"+missingSlug, tokNew, wrap("article", map[string]any{"body": "x"})).expect(404)
	r.call("articles.update.empty-title", "PUT", newPath, tokNew, wrap("article", map[string]any{"title": ""})).expect(422)
	r.call("articles.update.number-body", "PUT", newPath, tokNew, wrap("article", map[string]any{"body": 1})).expect(422)
	r.call("articles.update.no-wrapper", "PUT", newPath, tokNew, map[string]any{"body": "x"}).expect(422)
	x = r.call("articles.update.body", "PUT", newPath, tokNew, wrap("article", map[string]any{"body": "Updated body", "title": nil}))
	if x.expect(200) {
		checkArticle(x, "article", x.at("article"), true)
		eq(x, "article.slug", x.at("article", "slug"), newSlug)
		eq(x, "article.body", x.at("article", "body"), "Updated body")
		eq(x, "article.title", x.at("article", "title"), title)
		eq(x, "article.description", x.at("article", "description"), "Conformance description")
	}
	x = r.call("articles.update.title", "PUT", newPath, tokNew, wrap("article", map[string]any{"title": "  Conformance -- Renamed Title!  ", "description": "New description"}))
	renamed := ""
	if x.expect(200) {
		checkArticle(x, "article", x.at("article"), true)
		m := slugRe.FindStringSubmatch(newSlug)
		renamed = "conformance-renamed-title-" + m[1]
		eq(x, "article.slug", x.at("article", "slug"), renamed)
		eq(x, "article.title", x.at("article", "title"), "  Conformance -- Renamed Title!  ")
		eq(x, "article.description", x.at("article", "description"), "New description")
		eq(x, "article.body", x.at("article", "body"), "Updated body")
	}
	r.call("article.get.old-slug", "GET", newPath, "", nil).expect(404)
	if renamed == "" {
		panic(abort("article rename failed; cannot continue"))
	}
	renPath := "/api/articles/" + renamed
	x = r.call("article.get.renamed", "GET", renPath, tokNew, nil)
	if x.expect(200) {
		eq(x, "article.slug", x.at("article", "slug"), renamed)
	}
	r.call("favorite.created-article", "POST", renPath+"/favorite", tokV, nil).expect(200)
	x = r.call("comments.create.on-created-article", "POST", renPath+"/comments", tokV, wrap("comment", map[string]any{"body": "Comment by a seeded user"}))
	if x.expect(201) {
		checkComment(x, "comment", x.at("comment"))
		eq(x, "comment.author.username", x.at("comment", "author", "username"), f.userV)
		n, _ := num(x.at("comment", "id"))
		r.commentIDs[strconv.FormatInt(n, 10)] = true
	}
	x = r.call("article.get.renamed-after-favorite", "GET", renPath, tokV, nil)
	if x.expect(200) {
		eq(x, "article.favorited", x.at("article", "favorited"), true)
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), 1)
	}
	r.call("articles.delete.no-auth", "DELETE", renPath, "", nil).expect(401)
	r.call("articles.delete.not-author", "DELETE", renPath, tokA, nil).expect(403)
	r.call("articles.delete.unknown", "DELETE", "/api/articles/"+missingSlug, tokNew, nil).expect(404)
	r.call("articles.delete", "DELETE", renPath, tokNew, nil).expect(204)
	r.call("article.get.deleted", "GET", renPath, "", nil).expect(404)
	r.call("comments.list.deleted-article", "GET", renPath+"/comments", "", nil).expect(404)
	r.call("articles.delete.again", "DELETE", renPath, tokNew, nil).expect(404)
	x = r.call("articles.list.new-tag-after-delete", "GET", "/api/articles?tag="+newTag, "", nil)
	if x.expect(200) {
		if len(checkList(x)) != 0 || count(x) != 0 {
			x.fail("want an empty list after the delete")
		}
	}
	x = r.call("tags.list.after-delete", "GET", "/api/tags", "", nil)
	if x.expect(200) && len(arr(x.at("tags"))) != nTags+1 {
		x.fail("tags are never deleted: want %d", nTags+1)
	}

	// ===== the seeded author edits a seeded article (createdAt stays, updatedAt moves) =====
	x = r.call("articles.update.seeded-by-author", "PUT", xPath, tokA, wrap("article", map[string]any{"description": "Edited by the conformance run"}))
	if x.expect(200) {
		checkArticle(x, "article", x.at("article"), true)
		eq(x, "article.slug", x.at("article", "slug"), f.xSlug)
		eq(x, "article.description", x.at("article", "description"), "Edited by the conformance run")
		eq(x, "article.createdAt", x.at("article", "createdAt"), seed.ArticleCreated(f.X).UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z"))
		if str(x.at("article", "updatedAt")) <= str(x.at("article", "createdAt")) {
			x.fail("updatedAt must move forward")
		}
		eq(x, "article.favoritesCount", x.at("article", "favoritesCount"), favs0)
	}
}
