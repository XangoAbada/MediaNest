export function EmptyState({
  icon,
  title,
  description,
}: {
  icon: string;
  title: string;
  description?: string;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
      <div className="text-5xl opacity-40">{icon}</div>
      <div className="text-lg font-medium text-ink-dim">{title}</div>
      {description && (
        <div className="max-w-sm text-sm text-ink-faint">{description}</div>
      )}
    </div>
  );
}
