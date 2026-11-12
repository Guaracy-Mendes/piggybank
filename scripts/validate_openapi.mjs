import { createConfig, lint } from "@redocly/openapi-core";
const config = await createConfig({
  extends: ["spec"],
  rules: {
    struct: "error",
    "no-unresolved-refs": "error",
    "operation-operationId-unique": "error",
    "path-parameters-defined": "error",
  },
});
const results = await lint({
  ref: new URL("../contracts/openapi/implemented.json", import.meta.url)
    .pathname,
  config,
});
const failures = results.filter((problem) => problem.severity === "error");
if (failures.length) {
  for (const problem of failures)
    process.stderr.write(
      `${problem.ruleId}: ${problem.message} ${problem.location.map((location) => location.pointer).join(" ")}\n`,
    );
  process.exit(1);
}
process.stdout.write(
  "Implemented OpenAPI structural/reference/operation validation passed.\n",
);
