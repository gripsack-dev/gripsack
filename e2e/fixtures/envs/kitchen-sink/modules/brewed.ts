import { brew, module } from "@gripsack/core";

export default module("brewed", {
  fetch: brew("jq", "1.8.0"),
});
