import type { LucideIcon } from "lucide-react";

interface EmptySectionProps {
  icon: LucideIcon;
  title: string;
  body: string;
}

export function EmptySection({ icon: Icon, title, body }: EmptySectionProps) {
  return (
    <section className="empty-section">
      <div className="empty-icon"><Icon size={24} /></div>
      <h1>{title}</h1>
      <p>{body}</p>
    </section>
  );
}
