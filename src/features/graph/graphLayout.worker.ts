import { prepareLayouts, type GraphGrouping } from "./graphLayout";
import type { ResearchGraph } from "../../domain/graph";

self.onmessage = (
  event: MessageEvent<{ graph: ResearchGraph; grouping: GraphGrouping }>,
) => {
  self.postMessage(prepareLayouts(event.data.graph, event.data.grouping));
};
