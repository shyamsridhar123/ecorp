// Deliberately inert process-ownership fixture: no ECorp, database, runner or provider.
import http from 'node:http'

const bind = process.argv[process.argv.indexOf('--bind') + 1]
const endpoint = new URL('http://' + bind)
const started = Date.now()
const server = http.createServer((request, response) => {
  response.setHeader('content-type', 'application/json')
  response.end(JSON.stringify({ status: 'ok', runners: Date.now() - started > 250 ? 1 : 0 }))
})
server.listen(Number(endpoint.port), endpoint.hostname)
process.on('SIGTERM', () => { server.close(); server.closeAllConnections() })
