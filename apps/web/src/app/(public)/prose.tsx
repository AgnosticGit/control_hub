export function Prose({ title, lead, children }: { title: string; lead?: string; children: React.ReactNode }) {
  return (
    <article className="mx-auto max-w-3xl px-6 py-16">
      <h1 className="text-3xl font-semibold tracking-tight">{title}</h1>
      {lead ? <p className="mt-4 text-lg text-muted">{lead}</p> : null}
      <div className="mt-10 space-y-6 text-sm leading-relaxed text-text/90 [&_code]:rounded [&_code]:bg-surface-2 [&_code]:px-1.5 [&_code]:py-0.5 [&_code]:font-mono [&_h2]:mt-10 [&_h2]:text-xl [&_h2]:font-medium [&_li]:ml-5 [&_li]:list-disc [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:border [&_pre]:border-border [&_pre]:bg-surface [&_pre]:p-4 [&_pre]:font-mono [&_pre]:text-xs">
        {children}
      </div>
    </article>
  );
}
