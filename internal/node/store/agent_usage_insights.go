package store

import (
	"math"
	"math/bits"
	"slices"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

const exactUsageSamples = 20000

// After 20k measurements, use eight fixed sub-buckets per power of two.
// Quantiles then report the containing bucket's inclusive upper bound.
type usageDistribution struct {
	values  []uint64
	buckets *[496]uint64
	count   uint64
}

func latencyBucket(us uint64) int {
	shift := max(0, bits.Len64(us)-4)
	if shift == 0 {
		return int(us)
	}
	return (shift+1)*8 + int((us>>shift)&7)
}

func (d *usageDistribution) add(sample *uint64) {
	if sample == nil {
		return
	}
	d.count++
	if d.buckets == nil {
		if len(d.values) < exactUsageSamples {
			d.values = append(d.values, *sample)
			return
		}
		d.buckets = new([496]uint64)
		for _, v := range d.values {
			d.buckets[latencyBucket(v)]++
		}
		d.values = nil
	}
	d.buckets[latencyBucket(*sample)]++
}

func (d *usageDistribution) finish() protocol.UsagePercentiles {
	p := protocol.UsagePercentiles{Samples: d.count, Approximate: d.buckets != nil}
	if d.count == 0 {
		return p
	}
	slices.Sort(d.values)
	rank := func(percent uint64) *uint64 {
		n := (d.count*percent + 99) / 100
		if d.buckets == nil {
			v := d.values[n-1]
			return &v
		}
		for i, c := range d.buckets {
			if c < n {
				n -= c
				continue
			}
			v := uint64(i)
			if i >= 16 {
				v = uint64(9+i%8)<<uint(i/8-1) - 1
			}
			return &v
		}
		return nil
	}
	p.P50, p.P95, p.P99 = rank(50), rank(95), rank(99)
	return p
}

type usageInsightAccumulator struct {
	metrics        protocol.UsageInsights
	first, elapsed usageDistribution
}

func (a *usageInsightAccumulator) add(r protocol.UsageRequest, u canonical.Usage) error {
	m := &a.metrics
	m.Attempts++
	if r.Outcome != nil {
		switch *r.Outcome {
		case protocol.ConversationObservationOutcomeSucceeded:
			m.Succeeded++
		case protocol.ConversationObservationOutcomeFailed:
			m.Failed++
			kind := "unknown"
			if r.FailureKind != nil {
				kind = *r.FailureKind
			}
			if m.FailureKinds == nil {
				m.FailureKinds = map[string]uint64{}
			}
			m.FailureKinds[kind]++
		case protocol.ConversationObservationOutcomeCancelled:
			m.Cancelled++
		}
	}
	if r.Attempt != nil && *r.Attempt > 1 {
		m.Retries++
	}
	if r.EndKind != nil {
		if m.EndKinds == nil {
			m.EndKinds = map[string]uint64{}
		}
		m.EndKinds[string(*r.EndKind)]++
	}
	if u.CacheRead != nil && u.Input != nil {
		if math.MaxUint64-m.CacheRead < *u.CacheRead || math.MaxUint64-m.CacheInputTokens < *u.Input {
			return invalid("cache counters exceed the report limit")
		}
		m.CacheRead += *u.CacheRead
		m.CacheInputTokens += *u.Input
		m.CacheReadSamples++
	}
	if u.CacheWrite != nil {
		if math.MaxUint64-m.CacheWrite < *u.CacheWrite {
			return invalid("cache counters exceed the report limit")
		}
		m.CacheWrite += *u.CacheWrite
		m.CacheWriteSamples++
	}
	a.first.add(r.FirstTokenUs)
	a.elapsed.add(r.ElapsedUs)
	return nil
}

func (a *usageInsightAccumulator) finish() *protocol.UsageInsights {
	a.metrics.FirstTokenUs = a.first.finish()
	a.metrics.ElapsedUs = a.elapsed.finish()
	return &a.metrics
}
