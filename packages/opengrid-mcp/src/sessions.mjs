// One session per grid the model opened (#140). The server keeps each
// session's view as the truth: the model writes it with opengrid_set_view, the
// reader's changes arrive through opengrid_sync, and a rendered view picks up
// the model's when it next syncs (E40).
import { randomUUID } from "node:crypto";

export class Sessions {
  constructor() {
    this.sessions = new Map();
  }

  open(source, view, query, total) {
    const session = {
      id: randomUUID(),
      source,
      view,
      query,
      total,
      // Positions under `query`, as `opengrid-selection-change` names them.
      selected: [],
      // Bumped by every change; `origin` says whose it was.
      revision: 1,
      origin: "model",
    };
    this.sessions.set(session.id, session);
    return session;
  }

  get(id) {
    return this.sessions.get(id);
  }

  /** The model's view: the selection goes with the old one, as in the grid. */
  setView(session, view, query, total) {
    Object.assign(session, { view, query, total, selected: [], origin: "model" });
    session.revision += 1;
  }

  /** What the reader did, as the rendered grid reports it. */
  readerChanged(session, change) {
    Object.assign(session, {
      view: change.view ?? session.view,
      query: change.query ?? session.query,
      total: change.total ?? session.total,
      selected: change.selected ?? session.selected,
      origin: "reader",
    });
    session.revision += 1;
  }
}
