// Package seed defines the deterministic Conduit seed data set.
//
// Every value is a pure function of (kind, id), so the generator (cmd/seedgen) and the load
// generator (cmd/loadgen) agree on usernames, slugs, tags and authors without sharing files.
package seed

import (
	"fmt"
	"math"
	"math/rand/v2"
	"sort"
	"strings"
	"time"
)

const (
	NUsers    = 100_000
	NArticles = 500_000
	NTags     = 200

	Password = "password123"
	// bcrypt cost 10 of Password; every seeded user shares it (salts don't change login cost).
	PasswordHash = "$2a$10$ZSfEoPvlcsj2XfvsSeK25em4NZ9Jm7ntGqXuLgQfHEeZPYB8899wq"

	authorShare = 0.30
)

// Articles span two years ending at End; ids increase with created_at.
var (
	End   = time.Date(2026, 9, 1, 0, 0, 0, 0, time.UTC)
	Start = End.AddDate(-2, 0, 0)
	step  = End.Sub(Start) / NArticles
)

// Stream kinds: each (kind, id) pair has its own independent random stream.
const (
	kUser = iota + 1
	kAuthor
	kArticleMeta
	kArticleTitle
	kArticleText
	kFav
	kComment
	kFollow
	kTagName
)

// R returns the deterministic random stream for (kind, id).
func R(kind int, id int64) *rand.Rand {
	return rand.New(rand.NewPCG(uint64(kind)*0x9E3779B97F4A7C15, uint64(id)))
}

// ---------- words ----------

var (
	onsets = []string{"b", "c", "d", "f", "g", "h", "j", "k", "l", "m", "n", "p", "r", "s", "t", "v", "w", "z", "br", "ch", "cl", "dr", "gr", "pl", "sh", "st", "th", "tr"}
	vowels = []string{"a", "e", "i", "o", "u", "ai", "ea", "io", "ou"}
	codas  = []string{"", "", "", "n", "r", "s", "l", "t", "m", "x"}
)

// Word is a pronounceable pseudo-word of 1–3 syllables.
func Word(r *rand.Rand) string {
	var b strings.Builder
	n := 1 + r.IntN(3)
	for i := 0; i < n; i++ {
		b.WriteString(onsets[r.IntN(len(onsets))])
		b.WriteString(vowels[r.IntN(len(vowels))])
	}
	b.WriteString(codas[r.IntN(len(codas))])
	return b.String()
}

func words(r *rand.Rand, n int) []string {
	w := make([]string, n)
	for i := range w {
		w[i] = Word(r)
	}
	return w
}

// Sentence: 6–18 words, capitalized, with a full stop.
func Sentence(r *rand.Rand) string {
	w := words(r, 6+r.IntN(13))
	w[0] = strings.ToUpper(w[0][:1]) + w[0][1:]
	return strings.Join(w, " ") + "."
}

func sentences(r *rand.Rand, n int) string {
	s := make([]string, n)
	for i := range s {
		s[i] = Sentence(r)
	}
	return strings.Join(s, " ")
}

// ---------- tags ----------

var tagNames []string

// TagName returns tag i (1-based). Tag 1 is the most popular.
func TagName(i int) string { return tagNames[i-1] }

// ---------- users ----------

func Username(id int64) string {
	r := R(kUser, id)
	return fmt.Sprintf("%s%s%d", Word(r), Word(r), id)
}

func Email(id int64) string { return Username(id) + "@example.com" }

// UserBioImage returns nil pointers for NULL.
func UserBioImage(id int64) (bio, image *string) {
	r := R(kUser, id)
	Word(r)
	Word(r) // consume the username draws
	if r.Float64() < 0.7 {
		b := sentences(r, 1+r.IntN(2))
		bio = &b
	}
	if r.Float64() < 0.5 {
		im := fmt.Sprintf("https://i.pravatar.cc/150?u=%d", id)
		image = &im
	}
	return
}

// UserCreated: users join over the two years.
func UserCreated(id int64) time.Time {
	return Start.Add(time.Duration(float64(End.Sub(Start)) * float64(id-1) / NUsers))
}

// Authors are the 30% of users who write; Authors[i] is a user id.
var Authors []int64

// ---------- Zipf ----------

// Zipf samples 0..n-1 with P(k) ∝ 1/(k+1)^s, by inverse CDF.
type Zipf struct{ cdf []float64 }

func NewZipf(n int, s float64) *Zipf {
	c := make([]float64, n)
	sum := 0.0
	for k := 0; k < n; k++ {
		sum += 1 / math.Pow(float64(k+1), s)
		c[k] = sum
	}
	for k := range c {
		c[k] /= sum
	}
	return &Zipf{c}
}

func (z *Zipf) Sample(r *rand.Rand) int {
	u := r.Float64()
	return sort.SearchFloat64s(z.cdf, u)
}

var (
	authorZipf = (*Zipf)(nil) // over Authors, rank = index (a shuffled order, so popularity is not tied to id)
	tagZipf    = (*Zipf)(nil)
	// AuthorByRank is Authors in popularity order.
	AuthorByRank []int64
)

// SampleAuthor draws an author id with Zipf(0.8) popularity.
func SampleAuthor(r *rand.Rand) int64 { return AuthorByRank[authorZipf.Sample(r)] }

// SampleTag draws a tag index (1-based) with Zipf(1.0) popularity.
func SampleTag(r *rand.Rand) int { return tagZipf.Sample(r) + 1 }

// ---------- lognormal counts ----------

// LogNormalCount draws round(exp(N(mu, sigma))) capped at max, where mu gives the requested mean.
func LogNormalCount(r *rand.Rand, mean, sigma float64, max int) int {
	mu := math.Log(mean) - sigma*sigma/2
	v := int(math.Round(math.Exp(mu + sigma*r.NormFloat64())))
	if v > max {
		v = max
	}
	return v
}

// ---------- articles ----------

func Slugify(s string) string {
	var b strings.Builder
	dash := false
	for _, c := range strings.ToLower(s) {
		if (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') {
			b.WriteRune(c)
			dash = false
		} else if !dash {
			b.WriteByte('-')
			dash = true
		}
	}
	return strings.Trim(b.String(), "-")
}

func ArticleTitle(id int64) string {
	r := R(kArticleTitle, id)
	w := words(r, 3+r.IntN(6))
	for i := range w {
		w[i] = strings.ToUpper(w[i][:1]) + w[i][1:]
	}
	return strings.Join(w, " ")
}

func ArticleSlug(id int64) string { return Slugify(ArticleTitle(id)) + "-" + fmt.Sprint(id) }

func ArticleCreated(id int64) time.Time {
	r := R(kArticleMeta, id)
	r.Uint64() // author draw
	return Start.Add(step*time.Duration(id-1) + time.Duration(r.Int64N(int64(step))))
}

// ArticleAuthor: Zipf(0.8) over authors.
func ArticleAuthor(id int64) int64 {
	return SampleAuthor(R(kArticleMeta, id))
}

// ArticleTags returns 1–4 distinct tag indexes (1-based), sorted.
func ArticleTags(id int64) []int {
	r := R(kArticleMeta, id+1<<40)
	n := 1 + r.IntN(4)
	seen := map[int]bool{}
	var out []int
	for len(out) < n {
		t := SampleTag(r)
		if !seen[t] {
			seen[t] = true
			out = append(out, t)
		}
	}
	sort.Ints(out)
	return out
}

// ArticleText returns description and body (body: paragraphs separated by blank lines, ~1 KB on average).
func ArticleText(id int64) (description, body string) {
	r := R(kArticleText, id)
	description = Sentence(r)
	np := LogNormalCount(r, 3, 0.5, 12)
	if np < 1 {
		np = 1
	}
	ps := make([]string, np)
	for i := range ps {
		ps[i] = sentences(r, 2+r.IntN(4))
	}
	body = strings.Join(ps, "\n\n")
	return
}

// ---------- favorites / comments / follows ----------

// ArticleFavoriters returns the distinct users who favorited article id (mean ≈ 10, cap 2,000).
func ArticleFavoriters(id int64) []int64 {
	r := R(kFav, id)
	n := LogNormalCount(r, 10, 1.5, 2000)
	seen := make(map[int64]bool, n)
	out := make([]int64, 0, n)
	for len(out) < n {
		u := 1 + r.Int64N(NUsers)
		if !seen[u] {
			seen[u] = true
			out = append(out, u)
		}
	}
	return out
}

type Comment struct {
	Author  int64
	Body    string
	Created time.Time
}

// ArticleComments: mean ≈ 5, cap 200, within 30 days of the article (and before End).
func ArticleComments(id int64) []Comment {
	r := R(kComment, id)
	n := LogNormalCount(r, 5, 1.2, 200)
	created := ArticleCreated(id)
	window := 30 * 24 * time.Hour
	if rem := End.Sub(created); rem < window {
		window = rem
	}
	out := make([]Comment, n)
	for i := range out {
		out[i] = Comment{
			Author:  1 + r.Int64N(NUsers),
			Body:    sentences(r, 1+r.IntN(3)),
			Created: created.Add(time.Duration(r.Int64N(int64(window) + 1))),
		}
	}
	return out
}

// UserFollows: distinct authors user id follows (mean ≈ 20, cap 1,000), Zipf(0.8) popularity, never self.
func UserFollows(id int64) []int64 {
	r := R(kFollow, id)
	n := LogNormalCount(r, 20, 1.0, 1000)
	seen := make(map[int64]bool, n)
	out := make([]int64, 0, n)
	for tries := 0; len(out) < n && tries < n*20; tries++ {
		a := SampleAuthor(r)
		if a != id && !seen[a] {
			seen[a] = true
			out = append(out, a)
		}
	}
	return out
}

// ---------- init ----------

func init() {
	seen := map[string]bool{}
	tr := R(kTagName, 0)
	for len(tagNames) < NTags {
		w := Word(tr)
		if len(w) >= 4 && !seen[w] {
			seen[w] = true
			tagNames = append(tagNames, w)
		}
	}
	tagZipf = NewZipf(NTags, 1.0)

	for id := int64(1); id <= NUsers; id++ {
		if R(kAuthor, id).Float64() < authorShare {
			Authors = append(Authors, id)
		}
	}
	AuthorByRank = append([]int64(nil), Authors...)
	sr := R(kAuthor, 0)
	sr.Shuffle(len(AuthorByRank), func(i, j int) { AuthorByRank[i], AuthorByRank[j] = AuthorByRank[j], AuthorByRank[i] })
	authorZipf = NewZipf(len(AuthorByRank), 0.8)
}
