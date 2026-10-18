import assert from 'node:assert/strict'
import {test} from 'node:test'

import type {CommandMeta} from './types.ts'

import {buildArgv, missingRequired, shellQuote, splitArgs} from './argv.ts'

const flag = (name: string, extra: Partial<CommandMeta['flags'][number]> = {}): CommandMeta['flags'][number] => ({
  allowNo: false,
  char: null,
  default: null,
  description: null,
  kind: 'option',
  multiple: false,
  name,
  options: null,
  required: false,
  ...extra,
})
const arg = (name: string, required = false): CommandMeta['args'][number] => ({
  default: null,
  description: null,
  name,
  options: null,
  required,
})

const cmd = {
  args: [arg('key', true), arg('extra')],
  flags: [flag('toon', {kind: 'boolean'}), flag('header', {multiple: true}), flag('cache', {allowNo: true, kind: 'boolean'}), flag('profile', {required: true})],
}

test('builds flags then positionals', () => {
  const argv = buildArgv(cmd, {args: {key: 'SS-1'}, flags: {cache: false, header: 'A=1\n\nB=2', profile: 'default', toon: true}})
  assert.deepEqual(argv, ['--toon', '--header', 'A=1', '--header', 'B=2', '--no-cache', '--profile', 'default', 'SS-1'])
})

test('keeps positional gaps and protects dash values', () => {
  assert.deepEqual(buildArgv(cmd, {args: {extra: 'x'}, flags: {}}), ['', 'x'])
  assert.deepEqual(buildArgv(cmd, {args: {key: '-1'}, flags: {}}), ['--', '-1'])
})

test('reports missing required inputs', () => {
  assert.deepEqual(missingRequired(cmd, {args: {}, flags: {}}), ['key', '--profile'])
  assert.deepEqual(missingRequired(cmd, {args: {key: 'k'}, flags: {profile: 'p'}}), [])
})

test('quotes and splits round-trip', () => {
  const argv = ['jira', 'issue', 'search', '--jql', `project = "SS" AND summary ~ 'it''s'`, '']
  assert.deepEqual(splitArgs(shellQuote(argv)), argv)
  assert.deepEqual(splitArgs(`api call  "a b" c\\ d 'e'`), ['api', 'call', 'a b', 'c d', 'e'])
})
