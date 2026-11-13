// seedgen writes the deterministic Conduit seed as Postgres COPY text files (one set of part files per table).
//
//	seedgen -out /path/dir [-scale 1.0]
//
// -scale < 1 keeps only the first fraction of articles (and their favorites/comments) and of users' follows,
// for local development. Users and tags are always complete.
package main

import (
	"bufio"
	"flag"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"conduitbench/seed"
)

var esc = strings.NewReplacer("\\", "\\\\", "\t", "\\t", "\n", "\\n", "\r", "\\r")

func ts(t time.Time) string { return t.UTC().Format("2006-01-02 15:04:05.000000+00") }

func opt(s *string) string {
	if s == nil {
		return `\N`
	}
	return esc.Replace(*s)
}

type part struct {
	name   string
	lo, hi int64 // inclusive
}

func main() {
	out := flag.String("out", "seed-data", "output directory")
	scale := flag.Float64("scale", 1.0, "fraction of articles/follows to emit (dev only)")
	flag.Parse()
	if err := os.MkdirAll(*out, 0o755); err != nil {
		log.Fatal(err)
	}
	nArt := int64(float64(seed.NArticles) * *scale)
	nFollowUsers := int64(float64(seed.NUsers) * *scale)
	workers := runtime.NumCPU()

	write := func(path string, fn func(w *bufio.Writer)) {
		f, err := os.Create(path)
		if err != nil {
			log.Fatal(err)
		}
		w := bufio.NewWriterSize(f, 1<<20)
		fn(w)
		if err := w.Flush(); err != nil {
			log.Fatal(err)
		}
		f.Close()
	}

	t0 := time.Now()
	// tags
	write(filepath.Join(*out, "tags.tsv"), func(w *bufio.Writer) {
		for i := 1; i <= seed.NTags; i++ {
			fmt.Fprintf(w, "%d\t%s\n", i, seed.TagName(i))
		}
	})
	// users
	write(filepath.Join(*out, "users.tsv"), func(w *bufio.Writer) {
		for id := int64(1); id <= seed.NUsers; id++ {
			bio, img := seed.UserBioImage(id)
			fmt.Fprintf(w, "%d\t%s\t%s\t%s\t%s\t%s\t%s\n", id, seed.Username(id), seed.Email(id),
				seed.PasswordHash, opt(bio), opt(img), ts(seed.UserCreated(id)))
		}
	})

	// Article-keyed tables, split into ordered parts generated in parallel.
	split := func(n int64) []part {
		var ps []part
		chunk := (n + int64(workers) - 1) / int64(workers)
		for i, lo := 0, int64(1); lo <= n; i, lo = i+1, lo+chunk {
			hi := min(lo+chunk-1, n)
			ps = append(ps, part{fmt.Sprintf("%03d", i), lo, hi})
		}
		return ps
	}
	var wg sync.WaitGroup
	for _, p := range split(nArt) {
		wg.Add(1)
		go func(p part) {
			defer wg.Done()
			write(filepath.Join(*out, "articles."+p.name+".tsv"), func(w *bufio.Writer) {
				for id := p.lo; id <= p.hi; id++ {
					d, b := seed.ArticleText(id)
					c := ts(seed.ArticleCreated(id))
					fmt.Fprintf(w, "%d\t%s\t%d\t%s\t%s\t%s\t%s\t%s\n", id, seed.ArticleSlug(id), seed.ArticleAuthor(id),
						esc.Replace(seed.ArticleTitle(id)), esc.Replace(d), esc.Replace(b), c, c)
				}
			})
			write(filepath.Join(*out, "article_tags."+p.name+".tsv"), func(w *bufio.Writer) {
				for id := p.lo; id <= p.hi; id++ {
					for _, t := range seed.ArticleTags(id) {
						fmt.Fprintf(w, "%d\t%d\n", id, t)
					}
				}
			})
			write(filepath.Join(*out, "favorites."+p.name+".tsv"), func(w *bufio.Writer) {
				for id := p.lo; id <= p.hi; id++ {
					c := ts(seed.ArticleCreated(id))
					for _, u := range seed.ArticleFavoriters(id) {
						fmt.Fprintf(w, "%d\t%d\t%s\n", u, id, c)
					}
				}
			})
			// comments: no id column, so identity assigns ids in load order (parts are loaded in name order).
			write(filepath.Join(*out, "comments."+p.name+".tsv"), func(w *bufio.Writer) {
				for id := p.lo; id <= p.hi; id++ {
					for _, c := range seed.ArticleComments(id) {
						t := ts(c.Created)
						fmt.Fprintf(w, "%d\t%d\t%s\t%s\t%s\n", id, c.Author, esc.Replace(c.Body), t, t)
					}
				}
			})
		}(p)
	}
	for _, p := range split(nFollowUsers) {
		wg.Add(1)
		go func(p part) {
			defer wg.Done()
			write(filepath.Join(*out, "follows."+p.name+".tsv"), func(w *bufio.Writer) {
				for id := p.lo; id <= p.hi; id++ {
					c := ts(seed.UserCreated(id))
					for _, a := range seed.UserFollows(id) {
						fmt.Fprintf(w, "%d\t%d\t%s\n", id, a, c)
					}
				}
			})
		}(p)
	}
	wg.Wait()
	log.Printf("seed written to %s in %s (scale %.3f, %d articles)", *out, time.Since(t0).Round(time.Millisecond), *scale, nArt)
}
