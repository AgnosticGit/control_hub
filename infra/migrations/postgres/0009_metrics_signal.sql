-- OTLP metrics (gauges and sums from the Karbird agent, ADR-0018) are a regular ingest signal.
-- New tokens get it by default; tokens that already carried every signal of the old default get
-- it too, tokens deliberately narrowed to fewer signals stay as they are.

ALTER TABLE ingest_tokens ALTER COLUMN signals SET DEFAULT ARRAY['traces','logs','metrics','events'];

UPDATE ingest_tokens
SET signals = array_append(signals, 'metrics')
WHERE signals @> ARRAY['traces','logs','events']
  AND NOT signals @> ARRAY['metrics'];
