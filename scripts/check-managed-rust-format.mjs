import { spawnSync } from 'node:child_process';

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: process.cwd(),
    stdio: 'inherit',
    shell: false,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run('cargo', ['fmt', '-p', 'codex-task-supervisor', '--', '--check']);
run('rustfmt', [
  '--edition',
  '2021',
  '--check',
  'src-tauri/src/modules/codex_managed_task.rs',
  'src-tauri/src/modules/codex_task_store.rs',
  'src-tauri/src/modules/codex_task_supervisor.rs',
]);
