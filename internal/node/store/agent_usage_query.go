package store

// agentUsagePreset selects the built-in preset of the channel a response used:
// the one captured with its run or with the media model assigned to the
// session, else the channel as it is saved now.
const agentUsagePreset = `coalesce(
 CASE WHEN json_extract(captured,'$.id')=json_extract(provider,'$.id') AND json_extract(captured,'$.preset.source.kind')='builtin' THEN json_extract(captured,'$.preset.id') END,
 CASE WHEN json_extract(assigned,'$.id')=json_extract(provider,'$.id') AND json_extract(assigned,'$.preset.source.kind')='builtin' THEN json_extract(assigned,'$.preset.id') END,
 (SELECT json_extract(saved.body,'$.provider.preset.id') FROM providers saved WHERE saved.id=json_extract(provider,'$.id') AND json_extract(saved.body,'$.provider.preset.source.kind')='builtin'))`

const agentUsageFailure = `coalesce(json_extract(e.attempt,'$.failure_kind'),json_extract(e.attempt,'$.retry_reason'),CASE WHEN json_extract(e.attempt,'$.outcome')='failed' THEN 'unknown' END)`

const agentUsageProvider = `coalesce(json_extract(e.body,'$.metadata.execution.source.id'),
        CASE WHEN json_extract(e.body,'$.metadata.execution.media') IS NULL OR m.value IS NULL
        THEN json_extract(r.config,'$.provider') ELSE json_extract(m.value,'$.provider.id') END) AS provider_id,
    coalesce(json_extract(e.body,'$.metadata.execution.source.model'),
        CASE WHEN json_extract(e.body,'$.metadata.execution.media') IS NULL OR m.value IS NULL
        THEN json_extract(r.config,'$.model') ELSE json_extract(m.value,'$.model') END) AS model,
    coalesce(json_extract(e.body,'$.metadata.execution.source'),
        CASE WHEN json_extract(e.body,'$.metadata.execution.media') IS NULL OR m.value IS NULL
        THEN a.provider ELSE json_extract(m.value,'$.provider') END) AS provider`
const agentUsageJoins = `FROM agent_events e JOIN turns t ON t.id=e.turn
    JOIN agent_runs a ON a.turn=t.id
    JOIN session_revisions r ON r.session=t.session AND r.revision=t.revision
    JOIN sessions s ON s.id=t.session`
const agentUsageMedia = `LEFT JOIN session_media media ON media.session=t.session
    LEFT JOIN json_each(media.body) m ON m.key=json_extract(e.body,'$.metadata.execution.media')`

// A turn's revision, run and media assignment are stable in the read transaction.
// Capture their presentation and pricing facts once per (turn, source, media),
// rather than returning them through the SQL driver for every attempt.
const agentUsageScope = `WITH responses AS (SELECT e.sequence,s.project,r.worktree,json_extract(r.config,'$.resource') AS resource,
    json_extract(r.config,'$.assistant') AS assistant,a.provider AS captured,json_extract(m.value,'$.provider') AS assigned,
    ` + agentUsageProvider + ` ` + agentUsageJoins + ` ` + agentUsageMedia + ` WHERE e.sequence IN (SELECT CAST(value AS INTEGER) FROM json_each(?1)))
    SELECT e.sequence,e.provider_id,e.model,e.project,e.worktree,json_extract(e.provider,'$.api'),json_extract(e.provider,'$.credential.id'),
    json_extract(e.provider,'$.name'),coalesce(
        (SELECT name FROM projects WHERE id=e.project),
        CASE json_extract(e.resource,'$.kind')
            WHEN 'database' THEN (SELECT json_extract(body,'$.name') FROM database_profiles WHERE id=json_extract(e.resource,'$.id'))
            WHEN 'ssh' THEN (SELECT json_extract(body,'$.name') FROM ssh_profiles WHERE id=json_extract(e.resource,'$.id'))
        END, json_extract(e.resource,'$.id'),
        json_extract(e.assistant,'$.package.name') || ':' || json_extract(e.assistant,'$.id'),''),e.resource,e.assistant,` + agentUsagePreset + ` FROM responses e`

// Keep the legacy predicate identical to the partial usage index. V2 also admits
// observed attempts with no complete token report, without counting them as bills.
func agentUsageRows(observations, observedOnly, providerFilters bool) string {
	selection := `coalesce(json_type(e.body,'$.usage'),json_type(e.body,'$.summary.usage'))='object'
    AND coalesce(json_extract(e.body,'$.usage.input'),json_extract(e.body,'$.summary.usage.input'))>=0
    AND coalesce(json_extract(e.body,'$.usage.output'),json_extract(e.body,'$.summary.usage.output'))>=0`
	attempt, join := "NULL AS attempt,0 AS ordinal,1 AS single", ""
	if observations {
		attempt = "attempts.value AS attempt,coalesce(attempts.key,0) AS ordinal, json_extract(e.body,'$.metadata.execution.observation.attempts[1]') IS NULL AS single"
		join = "LEFT JOIN json_each(json_extract(e.body,'$.metadata.execution.observation.attempts')) attempts ON true"
		if observedOnly {
			selection = "json_type(e.body,'$.metadata.execution.observation.attempts[0]')='object'"
		} else {
			selection = "(" + selection + " OR json_type(e.body,'$.metadata.execution.observation.attempts[0]')='object')"
		}
	}
	// Omit provider/media resolution entirely in the usual unfiltered scan.
	provider, media := "NULL AS provider_id,NULL AS model,NULL AS provider", ""
	if providerFilters {
		provider, media = agentUsageProvider, agentUsageMedia
	}
	return `WITH responses AS (SELECT e.body,coalesce(json_extract(e.body,'$.usage'),json_extract(e.body,'$.summary.usage')) AS usage,e.sequence,e.session,e.turn,
	` + provider + `,` + attempt + ` ` + agentUsageJoins + ` ` + media + ` ` + join + ` WHERE ` + selection + `
    AND substr(json_extract(e.body,'$.created'),1,19)>=?1
    AND substr(json_extract(e.body,'$.created'),1,19)<=?2
    AND (?3='[]' OR s.project IN (SELECT CAST(value AS TEXT) FROM json_each(?3)))
    AND (?4='[]' OR r.worktree IN (SELECT CAST(value AS TEXT) FROM json_each(?4)))
    AND (?9='[]' OR e.session IN (SELECT CAST(value AS TEXT) FROM json_each(?9)))
    AND (CASE WHEN s.project IS NULL THEN coalesce(s.owner,(SELECT id FROM users WHERE local=1))=?14 ELSE EXISTS(SELECT 1 FROM projects access WHERE access.id=s.project AND (access.visibility='shared' OR coalesce(access.owner,(SELECT id FROM users WHERE local=1))=?14)) END))
    SELECT e.sequence,e.turn,e.session,json_extract(e.body,'$.metadata.execution.source'),json_extract(e.body,'$.metadata.execution.media'),
    json_extract(e.body,'$.created'),CASE WHEN e.single THEN e.usage END,CASE WHEN e.single THEN json_extract(e.body,'$.metadata.execution.timing') END,
    json_extract(e.body,'$.metadata.execution.observation.id'),e.ordinal,e.attempt
    FROM responses e
    WHERE (?5='[]' OR e.provider_id IN (SELECT CAST(value AS TEXT) FROM json_each(?5)))
    AND (?6='[]' OR e.model IN (SELECT CAST(value AS TEXT) FROM json_each(?6)))
    AND (?7='[]' OR json_extract(e.attempt,'$.outcome') IN (SELECT CAST(value AS TEXT) FROM json_each(?7)))
    AND (?8='[]' OR json_extract(e.attempt,'$.end_kind') IN (SELECT CAST(value AS TEXT) FROM json_each(?8)))
    AND (?10='[]' OR json_extract(e.provider,'$.credential.id') IN (SELECT CAST(value AS TEXT) FROM json_each(?10)))
    AND (?11='[]' OR json_extract(e.attempt,'$.status') IN (SELECT CAST(value AS INTEGER) FROM json_each(?11)))
    AND (?12='[]' OR ` + agentUsageFailure + ` IN (SELECT CAST(value AS TEXT) FROM json_each(?12)))
    AND (NOT ?15 OR json_extract(e.provider,'$.credential.id') IS NULL)
    AND (NOT ?16 OR e.attempt IS NOT NULL)
    AND (NOT ?17 OR coalesce(CASE WHEN e.single THEN json_extract(e.usage,'$.cache_read') END,
      CASE WHEN json_extract(e.attempt,'$.cache_reported') THEN json_extract(e.attempt,'$.usage.cached_input') END)=0)
    ORDER BY substr(json_extract(e.body,'$.created'),1,19),e.sequence LIMIT ?13`
}
