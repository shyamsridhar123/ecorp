// Test-only, read-only metadata observer. Original buffers are forwarded by
// the process supervisor, not rewritten here. Never emit paths or payloads.
export function createFsWireObserver(emit, maxFrameBytes = 16 * 1024 * 1024) {
  const pending = new Map()
  let sequence = 0
  const allowed = new Set(['sessionFs.stat', 'sessionFs.exists'])
  const idKey = (id) => `${typeof id}:${String(id)}`
  const validId = (id) => (typeof id === 'number' && Number.isSafeInteger(id)) ||
    (typeof id === 'string' && /^\d{1,20}$/u.test(id))

  function message(direction, value) {
    if (!value || typeof value !== 'object' || !validId(value.id)) return
    if (direction === 'cli' && allowed.has(value.method)) {
      if (pending.size >= 128) {
        emit({ phase: 'coverage', complete: false, reason: 'pending_request_bound' })
        return
      }
      const entry = { method: value.method, sequence: ++sequence, idType: typeof value.id, id: value.id }
      pending.set(idKey(value.id), entry)
      emit({ phase: 'request', method: entry.method, sequence: entry.sequence, id_type: entry.idType })
      return
    }
    if (direction !== 'sdk' || Object.hasOwn(value, 'method')) return
    let entry = pending.get(idKey(value.id))
    if (!entry) {
      const candidates = [...pending.values()].filter((item) => String(item.id) === String(value.id))
      if (candidates.length === 1) entry = candidates[0]
    }
    if (!entry) return
    pending.delete(idKey(entry.id))
    const result = value.result
    const object = result && typeof result === 'object' && !Array.isArray(result) ? result : {}
    const bool = (key) => typeof object[key] === 'boolean' ? object[key] : null
    const dateValid = (key) => typeof object[key] === 'string' && Number.isFinite(Date.parse(object[key]))
    emit({
      phase: 'response',
      method: entry.method,
      sequence: entry.sequence,
      id_type: typeof value.id,
      id_type_matches: entry.idType === typeof value.id,
      has_rpc_error: Object.hasOwn(value, 'error'),
      result_kind: result === null ? 'null' : Array.isArray(result) ? 'array' : typeof result,
      has_nested_stat: Object.hasOwn(object, 'stat'),
      has_nested_result: Object.hasOwn(object, 'result'),
      has_error: Object.hasOwn(object, 'error'),
      error_is_null: object.error === null,
      is_file_camel: bool('isFile'),
      is_directory_camel: bool('isDirectory'),
      is_file_snake: bool('is_file'),
      is_directory_snake: bool('is_directory'),
      exists: bool('exists'),
      mtime_type: typeof object.mtime,
      mtime_valid: dateValid('mtime'),
      birthtime_type: typeof object.birthtime,
      birthtime_valid: dateValid('birthtime'),
      size_type: typeof object.size,
    })
  }

  function decoder(direction) {
    let buffered = Buffer.alloc(0)
    let expected = null
    let discard = 0
    let disabled = false
    return (chunk) => {
      if (disabled) return
      buffered = Buffer.concat([buffered, chunk])
      while (buffered.length > 0) {
        if (discard > 0) {
          const count = Math.min(discard, buffered.length)
          buffered = buffered.subarray(count)
          discard -= count
          if (discard > 0) return
        }
        if (expected === null) {
          const end = buffered.indexOf('\r\n\r\n')
          if (end === -1) {
            if (buffered.length > 8192) {
              emit({ phase: 'coverage', complete: false, reason: 'invalid_or_oversized_header', direction })
              buffered = Buffer.alloc(0)
              disabled = true
            }
            return
          }
          const header = buffered.subarray(0, end).toString('ascii')
          buffered = buffered.subarray(end + 4)
          const length = header.match(/(?:^|\r\n)Content-Length:\s*(\d+)(?:\r\n|$)/iu)
          if (!length || !Number.isSafeInteger(Number(length[1]))) {
            emit({ phase: 'coverage', complete: false, reason: 'invalid_length', direction })
            disabled = true
            buffered = Buffer.alloc(0)
            return
          }
          expected = Number(length[1])
          if (expected > maxFrameBytes) {
            discard = expected
            expected = null
            emit({ phase: 'coverage', complete: false, reason: 'oversized_frame_skipped', direction })
            continue
          }
        }
        if (buffered.length < expected) return
        const body = buffered.subarray(0, expected)
        buffered = buffered.subarray(expected)
        expected = null
        try {
          message(direction, JSON.parse(body.toString('utf8')))
        } catch {
          emit({ phase: 'coverage', complete: false, reason: 'unparseable_frame', direction })
        }
      }
    }
  }
  return { fromCli: decoder('cli'), fromSdk: decoder('sdk') }
}
