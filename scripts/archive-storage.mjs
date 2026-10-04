// Local S3-compatible storage (SeaweedFS) for the log archive (ADR-0015).

export const ARCHIVE_BUCKET = "karbird-archive";

/**
 * SeaweedFS S3 identities, each limited to the archive bucket: an admin for maintenance, the
 * archiver and the API. SeaweedFS 3.97 refuses reads to identities without `Write` on the
 * bucket, so locally the API identity has it too; the API has no code path that writes or
 * deletes objects, and production gives it a read-only policy (`s3:GetObject`, `s3:ListBucket`).
 */
export function s3Identities(infra, bucket = ARCHIVE_BUCKET) {
  const account = { id: "karbird", displayName: "karbird" };
  const id = (name, prefix, actions) => ({
    name,
    credentials: [{ accessKey: infra[`CH_S3_${prefix}_ACCESS_KEY`], secretKey: infra[`CH_S3_${prefix}_SECRET_KEY`] }],
    account,
    actions,
  });
  for (const p of ["ADMIN", "ARCHIVER", "API"]) {
    if (!infra[`CH_S3_${p}_ACCESS_KEY`] || !infra[`CH_S3_${p}_SECRET_KEY`]) throw new Error(`missing CH_S3_${p} credentials`);
  }
  return {
    accounts: [{ ...account, emailAddress: "archive@karbird.invalid" }],
    identities: [
      id("admin", "ADMIN", ["Admin", "Read", "Write", "List", "Tagging"]),
      id("archiver", "ARCHIVER", [`Read:${bucket}`, `Write:${bucket}`, `List:${bucket}`]),
      id("api", "API", [`Read:${bucket}`, `Write:${bucket}`, `List:${bucket}`]),
    ],
  };
}
