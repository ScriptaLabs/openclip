-- Optional hardening for public.diagnostic_report. Not applied automatically: review it, then run it
-- in the Supabase SQL editor (or ask for it to be applied).

-- 1. Bound the damage if the public key is ever abused: at most 120 reports a minute overall.
create or replace function public.diagnostic_report_rate_limit()
returns trigger
language plpgsql
security definer
set search_path = ''
as $$
begin
  if (select count(*) from public.diagnostic_report where created_at > now() - interval '1 minute') >= 120 then
    raise exception 'too many reports, try again later' using errcode = 'P0001';
  end if;
  return new;
end;
$$;

revoke all on function public.diagnostic_report_rate_limit() from public, anon, authenticated;

create trigger diagnostic_report_rate_limit
  before insert on public.diagnostic_report
  for each statement
  execute function public.diagnostic_report_rate_limit();

-- 2. Retention: fixed or won't-fix reports are deleted after 30 days, everything else after 180.
create extension if not exists pg_cron with schema pg_catalog;

select cron.schedule(
  'diagnostic-report-retention',
  '17 3 * * *',
  $cron$
    delete from public.diagnostic_report
    where created_at < now() - interval '180 days'
       or (status in ('fixed', 'wontfix') and created_at < now() - interval '30 days');
  $cron$
);
