import type { LucideIcon } from "lucide-react";

interface EmptySectionProps {
  icon: LucideIcon;
  eyebrow: string;
  title: string;
  body: string;
}

export function EmptySection({ icon: Icon, eyebrow, title, body }: EmptySectionProps) {
  return (
    <section className="empty-section">
      <div className="empty-icon"><Icon size={24} /></div>
      <span className="eyebrow">{eyebrow}</span>
      <h1>{title}</h1>
      <p>{body}</p>
    </section>
  );
}

