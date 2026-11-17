UPDATE extraction_quality_profiles
SET active = 0
WHERE active = 1;

INSERT INTO extraction_quality_profiles (
    id,
    version,
    rules_json,
    active,
    created_at
) VALUES (
    'extraction-quality-v1.1',
    '1.1.0',
    '{"id":"extraction-quality-v1.1","version":"1.1.0","requireSearchableText":true,"flatPdfTextIsDegraded":true,"maximumCoverageDiscrepancy":0.15}',
    1,
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
);
