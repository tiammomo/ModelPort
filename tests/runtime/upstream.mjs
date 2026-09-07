import { setTimeout as delay } from 'node:timers/promises'
import { body, json, server } from './support.mjs'

export async function upstream(t) {
  let mode = 'normal'
  let calls = 0
  let active = 0
  let peak = 0
  const mock = await server(async (req, res) => {
    if (req.url === '/v1/models') return json(res, { data: [{ id: 'assurance-model' }] })
    if (req.url !== '/v1/chat/completions') { res.writeHead(404); return res.end() }
    calls++
    active++
    peak = Math.max(peak, active)
    let released = false
    res.on('close', () => { if (!released) { active--; released = true } })
    const input = JSON.parse(await body(req))
    const toolResult = input.messages.some(message => message.role === 'tool')
    const tool = input.tools?.[0]?.function
    const choice = tool && !toolResult
      ? { role: 'assistant', content: null, tool_calls: [{ id: 'assurance_call', type: 'function', function: { name: tool.name, arguments: '{"city":"Shanghai"}' } }] }
      : { role: 'assistant', content: 'OK' }
    if (mode === 'error') return json(res, { error: { message: 'synthetic unavailable' } }, 503)
    if (!input.stream) {
      await delay(20)
      return json(res, { id: 'assurance_completion', object: 'chat.completion', model: 'assurance-model', created: 1,
        choices: [{ index: 0, message: choice, finish_reason: choice.tool_calls ? 'tool_calls' : 'stop' }],
        usage: { prompt_tokens: 5, completion_tokens: 2, total_tokens: 7 } })
    }
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    const chunk = (delta, finish = null) => res.write(`data: ${JSON.stringify({
      id: 'assurance_completion', object: 'chat.completion.chunk', created: 1, model: 'assurance-model',
      choices: [{ index: 0, delta, finish_reason: finish }],
    })}\n\n`)
    chunk({ role: 'assistant' })
    if (choice.tool_calls) {
      chunk({ tool_calls: [{ index: 0, ...choice.tool_calls[0], function: { name: tool.name, arguments: '{"city":' } }] })
      chunk({ tool_calls: [{ index: 0, function: { arguments: '"Shanghai"}' } }] })
    } else chunk({ content: 'OK' })
    if (mode === 'hold') return
    await delay(100)
    if (mode === 'truncated') return res.end()
    chunk({}, choice.tool_calls ? 'tool_calls' : 'stop')
    res.end('data: [DONE]\n\n')
  })
  t.after(() => mock.close())
  return { ...mock, setMode: value => { mode = value }, calls: () => calls, active: () => active, peak: () => peak }
}
