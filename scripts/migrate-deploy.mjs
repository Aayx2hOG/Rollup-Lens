const maxAttempts = 3;

for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
  const child = Bun.spawn(["bunx", "prisma", "migrate", "deploy"], {
    cwd: import.meta.dir + "/..",
    env: Bun.env,
    stdout: "inherit",
    stderr: "pipe",
  });
  const stderr = await new Response(child.stderr).text();
  const exitCode = await child.exited;

  if (stderr) {
    await Bun.write(Bun.stderr, stderr);
  }
  if (exitCode === 0) {
    process.exitCode = 0;
    break;
  }

  const lockTimedOut = stderr.includes("P1002") || stderr.includes("advisory lock");
  if (!lockTimedOut || attempt === maxAttempts) {
    process.exitCode = exitCode;
    break;
  }

  const delaySeconds = attempt * 12;
  console.warn(
    `Migration lock is busy. Retrying in ${delaySeconds} seconds (${attempt + 1}/${maxAttempts})...`,
  );
  await Bun.sleep(delaySeconds * 1000);
}
