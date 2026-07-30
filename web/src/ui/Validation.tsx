/**
 * Compiler diagnostics.
 *
 * These come straight from `cf-compile`'s `CompileWarning` set — the same
 * values the engine acts on. Grouping by severity rather than by element means
 * the thing that blocks simulation is always at the top.
 */

import { useApp } from '../state/store';

export function Validation() {
  const warnings = useApp((s) => s.warnings);
  const fatal = warnings.filter((w) => w.fatal);
  const advisory = warnings.filter((w) => !w.fatal);

  return (
    <section className="panel validation">
      <h2 className="panel-title">
        Validation
        <span className="panel-count">{warnings.length}</span>
      </h2>

      {warnings.length === 0 && (
        <p className="empty">No issues. The venue compiles cleanly.</p>
      )}

      {fatal.length > 0 && (
        <ul className="issues">
          {fatal.map((w, i) => (
            <li key={`f${i}`} className="issue is-fatal">
              <span className="issue-code">{w.code}</span>
              <span className="issue-msg">{w.message}</span>
            </li>
          ))}
        </ul>
      )}

      {advisory.length > 0 && (
        <ul className="issues">
          {advisory.map((w, i) => (
            <li key={`a${i}`} className="issue">
              <span className="issue-code">{w.code}</span>
              <span className="issue-msg">{w.message}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
