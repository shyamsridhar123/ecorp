import type { MissionResultDeliverable, MissionResultPresentation } from './missionResultContext'
import { WorkResultCard } from './WorkResultCard'

type Props = {
  result: MissionResultPresentation
  busy?: boolean
  onRefresh: () => void
  onDownload?: (deliverable: MissionResultDeliverable) => void
  onSelectDelivered?: () => void
}

/** Links are supplied only by the consistency-checked, room-authorized reader. */
export function PublishedResultCard({
  result, busy = false, onRefresh, onDownload, onSelectDelivered,
}: Props) {
  if (result.state === 'loading') {
    return <WorkResultCard heading="Finding the result" status="Loading" pending
      description="Checking the exact delivery record for this work item." />
  }
  if (result.state === 'unavailable') {
    return <WorkResultCard heading="Result details are unavailable" status="Check connection" tone="attention"
      description="The delivery record could not be confirmed. Existing evidence and history remain available below."
      actions={<button type="button" className="button button-secondary" onClick={onRefresh}>Refresh result</button>} />
  }
  if (result.state === 'mismatch') {
    return <WorkResultCard heading="An earlier run is selected" status="Historical evidence"
      description="This review stays on the run you selected. Switch explicitly to inspect the work item’s delivered result."
      actions={onSelectDelivered
        ? <button type="button" className="button button-primary" onClick={onSelectDelivered}>Go to delivered result</button>
        : <button type="button" className="button button-secondary" onClick={onRefresh}>Refresh result</button>}>
      {!onSelectDelivered ? <p className="work-result-notice">The delivered run is not available in this view. No other run has been selected.</p> : null}
    </WorkResultCard>
  }
  const publication = result.publication
  if (!publication) {
    return <WorkResultCard heading="No pull request recorded yet" status="Not published"
      description="Verified source can be reviewed before publication. A trusted publisher handles GitHub delivery; opening this card starts no new work."
      actions={<button type="button" className="button button-secondary" onClick={onRefresh}>Refresh result</button>} />
  }
  const hasLink = Boolean(result.pullRequestUrl)
  const failed = result.state === 'failed'
  const published = result.state === 'published'
  const heading = failed ? 'Publication needs attention'
    : published ? 'Your pull request is ready'
      : hasLink ? 'Pull request created' : 'Preparing the pull request'
  const description = failed
    ? hasLink
      ? 'A pull request exists, but publication has not finished. Open it to inspect the result; no new run is needed to view it.'
      : 'Publication has not finished. The existing source and verification history are preserved.'
    : published
      ? 'Open the verified result on GitHub. Publication does not merge or deploy the application.'
      : hasLink
        ? 'The pull request is available. ECorp is still finishing the publication record.'
        : 'ECorp has a publication record, but no pull request link has been confirmed yet.'
  return (
    <WorkResultCard
      heading={heading}
      status={failed ? 'Needs attention' : published ? 'Published' : 'Publishing'}
      tone={failed ? 'attention' : published ? 'success' : 'working'}
      description={description}
      actions={<>
        {result.pullRequestUrl ? (
          <a className="button button-primary" href={result.pullRequestUrl}
            target="_blank" rel="noopener noreferrer">
            Open pull request #{publication.pull_request_number}
          </a>
        ) : null}
        {result.deliverable && onDownload ? (
          <button type="button" className="button button-secondary"
            disabled={busy}
            onClick={() => result.deliverable && onDownload(result.deliverable)}>
            Download source bundle
          </button>
        ) : null}
        {!hasLink ? <button type="button" className="button button-secondary" onClick={onRefresh}>Refresh result</button> : null}
      </>}
      facts={[
        { label: 'Repository', value: `${publication.target_repository} · ${publication.base_ref}` },
        { label: 'Verified commit', value: <code>{publication.commit_sha.slice(0, 12)}</code> },
      ]}
      details={<>
        <p><strong>Delivery branch</strong><br /><code>{publication.branch}</code></p>
        <p><strong>Delivered run</strong><br /><code>{publication.run_id}</code></p>
        <p><strong>Publication record</strong><br /><code>{publication.id}</code> · version {publication.version}</p>
        {publication.pull_request_draft !== null ? <p>Recorded PR: {publication.pull_request_draft ? 'draft' : publication.pull_request_state ?? 'available'}.</p> : null}
        <p>A source bundle is a portable ECorp artifact, not a running preview. No application URL has been inferred.</p>
        <p>The server checks source-download access, integrity and retention when you download.</p>
      </>}
    />
  )
}
