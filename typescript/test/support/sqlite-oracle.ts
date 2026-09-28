// Office browser specs live outside this package and must not resolve root-hoisted
// test dependencies; they reach the tooling-owned SQLite oracle through this module.
export { default } from 'better-sqlite3';
