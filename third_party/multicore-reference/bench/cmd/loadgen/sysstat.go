package main

import (
	"bytes"
	"os"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// cpuSampler reads the whole box's CPU from /proc/stat (Linux). On other systems ok() is false and CPU is N/A.
type cpuSampler struct {
	busy, total uint64
	ok          bool
}

func readProcStat() (busy, total uint64, ok bool) {
	b, err := os.ReadFile("/proc/stat")
	if err != nil {
		return 0, 0, false
	}
	line, _, _ := bytes.Cut(b, []byte("\n"))
	if !strings.HasPrefix(string(line), "cpu ") {
		return 0, 0, false
	}
	busy, total = statLine(string(line)) // user nice system idle iowait irq softirq steal; busy excludes idle+iowait
	return busy, total, total > 0
}

func newCPUSampler() *cpuSampler {
	s := &cpuSampler{}
	s.busy, s.total, s.ok = readProcStat()
	return s
}

// sample returns busy % of the whole box since the previous call, or nil where unavailable.
func (s *cpuSampler) sample() *float64 {
	if !s.ok {
		return nil
	}
	b, t, ok := readProcStat()
	if !ok || t <= s.total {
		return nil
	}
	pct := 100 * float64(b-s.busy) / float64(t-s.total)
	s.busy, s.total = b, t
	return &pct
}

// procCPU returns this process's user+system CPU time.
func procCPU() time.Duration {
	var ru syscall.Rusage
	if syscall.Getrusage(syscall.RUSAGE_SELF, &ru) != nil {
		return 0
	}
	return time.Duration(ru.Utime.Nano() + ru.Stime.Nano())
}

// rssMB returns the current resident set size (Linux), else the peak RSS from getrusage.
func rssMB() float64 {
	if b, err := os.ReadFile("/proc/self/statm"); err == nil {
		f := strings.Fields(string(b))
		if len(f) > 1 {
			pages, _ := strconv.ParseFloat(f[1], 64)
			return pages * float64(os.Getpagesize()) / (1 << 20)
		}
	}
	return peakRSSMB()
}

func peakRSSMB() float64 {
	var ru syscall.Rusage
	if syscall.Getrusage(syscall.RUSAGE_SELF, &ru) != nil {
		return 0
	}
	if runtime.GOOS == "darwin" {
		return float64(ru.Maxrss) / (1 << 20) // bytes
	}
	return float64(ru.Maxrss) / 1024 // KB
}
