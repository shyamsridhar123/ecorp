import http from 'node:http'

const port = Number(process.env.FAKE_OIDC_PORT ?? 8792)
const issuer = `http://127.0.0.1:${port}`

const users = new Map([
  ['alice-token', { sub: 'alice-subject', email: 'alice@example.test' }],
  ['bob-token', { sub: 'bob-subject', email: 'bob@example.test' }],
  ['unknown-token', { sub: 'unknown-subject', email: 'unknown@example.test' }],
])

const server = http.createServer((request, response) => {
  if (request.url === '/.well-known/openid-configuration') {
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify({
      issuer,
      userinfo_endpoint: `${issuer}/userinfo`,
    }))
    return
  }
  if (request.url === '/userinfo') {
    const token = request.headers.authorization?.replace(/^Bearer /, '')
    const user = token ? users.get(token) : undefined
    if (!user) {
      response.writeHead(401, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ error: 'invalid_token' }))
      return
    }
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify(user))
    return
  }
  response.writeHead(404)
  response.end()
})

server.listen(port, '127.0.0.1', () => {
  process.stdout.write(`fake OIDC listening on ${issuer}\n`)
})
