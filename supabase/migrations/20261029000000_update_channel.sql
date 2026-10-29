-- OpenClip release channel.
--
-- CI inserts one row per release into public.update; installed copies subscribe to those inserts
-- (Supabase Realtime) and check for the new version immediately. The row is only a push signal.
-- What actually installs is decided by the signed manifest (latest.json), which the app verifies
-- against the public key built into it.
--
-- Run this once in the Supabase SQL editor (the table itself already exists).

-- `src` carries the release: { version, notes, pub_date, channel, installer, installer_url,
-- github_url, sha256, commit }.
alter table public.update enable row level security;

-- Anyone running OpenClip may read and subscribe to releases. They hold only the public anon key.
drop policy if exists "Anyone can read releases" on public.update;
create policy "Anyone can read releases"
  on public.update
  for select
  to anon, authenticated
  using (true);

-- Deliberately no insert/update/delete policy for anon or authenticated users. Only the CI key,
-- which bypasses row level security, can write. Keep it out of the app and out of the repository.

-- Newest-first lookups (the app and any dashboards ask for "the latest").
create index if not exists update_created_at_idx on public.update (created_at desc);

-- Turn on realtime delivery for inserts.
do $$
begin
  if not exists (
    select 1 from pg_publication_tables
    where pubname = 'supabase_realtime' and schemaname = 'public' and tablename = 'update'
  ) then
    alter publication supabase_realtime add table public.update;
  end if;
end $$;
