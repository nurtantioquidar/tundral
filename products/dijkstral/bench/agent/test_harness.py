"""Tests for the agent-outcome runner (run.py, events.py, score.py). Stdlib unittest; no model,
no Codex process, no network.

    uv run --no-project python -m unittest discover -s bench/agent -p 'test_*.py'

`fixtures/codex-events-real.jsonl` is the event stream of one real `codex exec --json` call
(codex-cli 0.162.1) on a two-line scratch task; `codex-events-fake-mcp.jsonl` is real Codex
driven by the local fake provider (a shell command and a successful MCP call);
`codex-events-quota.jsonl` and `codex-events-transient.jsonl` are hand-written in the same shape.
"""

from __future__ import annotations

import json
import os
import shutil
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
FIX = HERE / "fixtures"

import events
import run
import score


def lines(name: str) -> list[str]:
    return (FIX / name).read_text().splitlines()


def task(**kw) -> dict:
    t = {
        "id": "fastapi-k01",
        "suite": "keyfacts",
        "corpus_id": "fastapi",
        "corpus": "/nonexistent/fastapi",
        "semantic": False,
        "question": "Where are API routes registered?",
        "question_date": None,
        "kind": "locate",
        "reference": "fastapi/routing.py, APIRouter.add_api_route",
        "facts": [{"fact": "fastapi/routing.py", "aliases": ["routing.py"]}],
    }
    t.update(kw)
    return t


class EventParsing(unittest.TestCase):
    def test_real_call(self):
        p = events.parse_events(lines("codex-events-real.jsonl"))
        self.assertEqual(p["answer"], "PELICAN yes")
        self.assertEqual(p["thread_id"], "01a14530-08ca-7260-a117-19ca8476d089")
        self.assertEqual(
            p["usage"],
            {
                "input_tokens": 45021,
                "cached_input_tokens": 41856,
                "cache_write_input_tokens": 0,
                "output_tokens": 126,
                "reasoning_output_tokens": 0,
            },
        )
        self.assertEqual(p["tool_calls"], 2)
        self.assertEqual(p["commands"], 1)
        self.assertEqual(p["commands_by_program"], {"cat": 1})
        self.assertEqual(p["mcp_calls_by_tool"], {"dijkstral.graph_stats": 1})
        self.assertEqual(
            p["mcp_failed"], 1
        )  # "requires approval, but approval policy is never"
        self.assertEqual(len(p["errors"]), 2)  # two non-fatal warning items
        self.assertEqual(p["fatal"], [])
        self.assertEqual(events.classify(p, 0, ""), "ok")

    def test_real_judge_call(self):
        p = events.parse_events(lines("codex-events-judge-real.jsonl"))
        self.assertEqual(events.classify(p, 0, ""), "ok")
        self.assertEqual(p["tool_calls"], 0)
        self.assertEqual(p["usage"]["input_tokens"], 12222)
        self.assertEqual(run.parse_verdict(p["answer"])["correct"], True)  # ty: ignore[not-subscriptable]

    def test_fake_provider_mcp_call(self):
        p = events.parse_events(lines("codex-events-fake-mcp.jsonl"))
        self.assertEqual(p["answer"], "FAKE ANSWER")
        self.assertEqual(p["mcp_calls"], 1)
        self.assertEqual(p["mcp_failed"], 0)
        self.assertEqual(p["usage"]["input_tokens"], 300)
        self.assertEqual(p["usage"]["reasoning_output_tokens"], 3)

    def test_started_items_are_not_counted_and_bad_lines_skipped(self):
        p = events.parse_events(
            [
                '{"type":"item.started","item":{"type":"command_execution","command":"ls"}}',
                "not json",
                "",
                '{"type":"item.completed","item":{"type":"command_execution","command":"ls"}}',
            ]
        )
        self.assertEqual(p["commands"], 1)
        self.assertEqual(p["bad_lines"], 1)
        self.assertIsNone(p["usage"])

    def test_command_program(self):
        cp = events.command_program
        self.assertEqual(cp("/bin/zsh -lc 'rg -n foo | head'"), "rg")
        self.assertEqual(cp("/bin/bash -lc \"cd src && sed -n '1,5p' a.py\""), "sed")
        self.assertEqual(cp("/bin/zsh -lc 'FOO=1 graphify query x'"), "graphify")
        self.assertEqual(cp("ls -la"), "ls")
        p = events.parse_events(
            [
                '{"type":"item.completed","item":{"type":"command_execution","command":"/bin/zsh -lc \'dijkstral query \\"router\\"\'"}}'
            ]
        )
        self.assertEqual(p["graph_cli_commands"], {"dijkstral": 1})


class Classification(unittest.TestCase):
    def test_quota_from_events(self):
        p = events.parse_events(lines("codex-events-quota.jsonl"))
        self.assertEqual(events.classify(p, 1, ""), "quota")

    def test_quota_from_stderr_only(self):
        p = events.parse_events([])
        stderr = "2026-10-16T14:50:46Z ERROR codex_core: unexpected status 429 Too Many Requests: usage_limit_reached\n"
        self.assertEqual(events.classify(p, 1, stderr), "quota")

    def test_transient(self):
        p = events.parse_events(lines("codex-events-transient.jsonl"))
        self.assertEqual(events.classify(p, 1, ""), "transient")
        ok = events.parse_events(lines("codex-events-real.jsonl"))
        self.assertEqual(
            events.classify(ok, 1, ""), "transient"
        )  # answer but non-zero exit
        self.assertEqual(events.classify(ok, 0, "", timed_out=True), "transient")
        self.assertEqual(
            events.classify(events.parse_events([]), 0, ""), "transient"
        )  # no answer

    def test_quota_words_in_an_answer_do_not_count(self):
        p = events.parse_events(
            [
                '{"type":"item.completed","item":{"type":"agent_message","text":"The rate limit is 429 per hour."}}'
            ]
        )
        self.assertEqual(events.classify(p, 0, ""), "ok")


class Facts(unittest.TestCase):
    def m(self, fact: str, answer: str, aliases: list[str] | None = None) -> bool:
        return score.fact_matches({"fact": fact, "aliases": aliases or []}, answer)

    def test_identifiers_need_identifier_boundaries(self):
        self.assertFalse(self.m("get_openapi", "it calls get_openapi_path"))
        self.assertTrue(
            self.m("get_openapi", "it calls `get_openapi()` then get_openapi_path")
        )
        self.assertFalse(self.m("updatePod", "see updatePodInSchedulingQueue"))
        self.assertTrue(self.m("updatePod", "PriorityQueue.updatePod moves it"))
        self.assertTrue(self.m("updatePod", "calls updatePod() on the queue"))
        self.assertTrue(self.m("search_path", "in mod::search_path"))
        self.assertFalse(self.m("search_path", "the search_paths list"))
        self.assertFalse(self.m("search_path", "default_search_path is used"))
        self.assertFalse(self.m("Preempt", "DefaultPreemption plugin"))
        self.assertTrue(self.m("Preempt", "DefaultPreemption.Preempt runs"))
        self.assertTrue(self.m("get_request_handler()", "inside get_request_handler"))

    def test_identifiers_are_case_sensitive(self):
        self.assertFalse(
            self.m("RequestValidationError", "raises a requestvalidationerror")
        )
        self.assertTrue(
            self.m("RequestValidationError", "raises a **RequestValidationError**")
        )

    def test_paths(self):
        f = "fastapi/routing.py"
        self.assertTrue(self.m(f, "See `fastapi\\routing.py`."))
        self.assertTrue(self.m(f, "in ./fastapi/routing.py"))
        self.assertTrue(self.m(f, "fastapi/routing.py:120 defines it"))
        self.assertTrue(self.m(f, "It is in fastapi/routing.py."))
        self.assertTrue(self.m(f, "[routing](fastapi/routing.py)"))
        self.assertFalse(self.m(f, "fastapi/routing.pyc"))
        self.assertFalse(self.m(f, "docs/fastapi/routing.py"))
        self.assertFalse(self.m(f, "FastAPI/Routing.py"))
        self.assertTrue(self.m(f, "the routing.py module", aliases=["routing.py"]))
        self.assertFalse(
            self.m(
                "fastapi/utils.py",
                "fastapi/dependencies/utils.py",
                aliases=["utils.py"],
            )
        )
        self.assertFalse(self.m("schedule_one.go", "the xschedule_one.go file"))
        self.assertTrue(self.m("crates/searcher", "the crates/searcher/ crate"))
        self.assertTrue(self.m("crates/searcher", "crates/searcher/src/lib.rs"))
        self.assertFalse(self.m("crates/searcher", "crates/searcher-extra"))
        self.assertFalse(self.m("fastapi/routing.py", "fastapi/routing.py/x"))
        self.assertTrue(
            self.m(
                "crates/searcher",
                "lives in crates/searcher, see",
                aliases=["grep-searcher"],
            )
        )
        self.assertTrue(
            self.m(
                "crates/searcher", "the grep-searcher crate", aliases=["grep-searcher"]
            )
        )

    def test_recall_and_min_facts(self):
        facts = [{"fact": "a.py"}, {"fact": "b.py"}, {"fact": "c.py"}]
        s = score.score_facts(facts, "a.py and b.py")
        self.assertAlmostEqual(s["recall"], 2 / 3)
        self.assertFalse(s["pass"])
        self.assertEqual(s["missing"], ["c.py"])
        self.assertTrue(score.score_facts(facts, "a.py and b.py", min_facts=2)["pass"])
        self.assertFalse(
            score.score_facts(facts, "a.py and b.py", min_facts=0)["pass"]
        )  # 0 = all
        self.assertIsNone(score.score_facts([], "x")["pass"])

    def test_committed_question_sets_match_their_own_references(self):
        """Each fact of the committed key-fact sets is credited by an answer that names it."""
        for path in sorted((HERE / "questions").glob("*.jsonl")):
            for t in run.load_tasks(path):
                for f in t["facts"]:
                    self.assertTrue(
                        score.fact_matches(f, f"Answer: {f['fact']}."), (t["id"], f)
                    )
                    self.assertTrue(
                        score.fact_matches(f, f"`{f['fact']}`"), (t["id"], f)
                    )


class SuiteScores(unittest.TestCase):
    def test_memory_suites_use_memory_py(self):
        loc = task(
            suite="locomo",
            kind="adversarial",
            reference="Not mentioned in the conversation",
            facts=[],
        )
        self.assertEqual(
            run.suite_score(loc, "That is not mentioned in the conversation."), 1.0
        )
        self.assertEqual(run.suite_score(loc, "She went to Paris."), 0.0)
        lme = task(
            suite="longmemeval",
            kind="multi-session",
            reference="three",
            facts=[],
            adversarial=True,
        )
        self.assertEqual(run.suite_score(lme, "You never mentioned that."), 1.0)
        lme["adversarial"] = False
        self.assertEqual(run.suite_score(lme, "three"), 1.0)
        f1 = run.suite_score(
            task(suite="locomo", kind="single-hop", reference="7 May 2023", facts=[]),
            "On 7 May 2023.",
        )
        self.assertGreater(f1, 0.8)


class Prompts(unittest.TestCase):
    def test_prompt_shows_only_question_and_date(self):
        t = task(
            suite="longmemeval",
            question="What degree did I graduate with?",
            question_date="2023/05/30 (Tue) 23:40",
            reference="SECRET-REFERENCE",
            source_id="e47becba_abs",
            distractor="SECRET-DISTRACTOR",
        )
        p = run.build_prompt(t)
        self.assertIn("What degree did I graduate with?", p)
        self.assertIn("Question date: 2023/05/30 (Tue) 23:40", p)
        for secret in (
            "SECRET",
            "e47becba",
            "_abs",
            "fastapi",
            "graphify",
            "dijkstral",
        ):
            self.assertNotIn(secret, p)
        self.assertTrue(p.startswith("Question:"))
        self.assertIn("these notes", p)
        self.assertIn("this repository", run.build_prompt(task()))

    def test_judge_prompts_hide_ids_and_distractors(self):
        loc = task(
            suite="locomo",
            kind="adversarial",
            source_id="conv-26/qa/7",
            distractor="SECRET-DISTRACTOR",
            facts=[],
        )
        prompt, structured = run.judge_spec(loc, "an answer")
        self.assertTrue(structured)
        self.assertNotIn("SECRET", prompt)
        self.assertNotIn("conv-26", prompt)
        self.assertIn("an answer", prompt)
        lme = task(
            suite="longmemeval",
            kind="temporal-reasoning",
            source_id="abc_abs",
            adversarial=True,
            facts=[],
        )
        prompt, structured = run.judge_spec(lme, "I can't find that.")
        self.assertFalse(structured)
        self.assertIn("unanswerable", prompt)
        self.assertNotIn("abc_abs", prompt)
        kf_prompt, _ = run.judge_spec(task(), "x")
        self.assertIn("Key facts", kf_prompt)
        self.assertIn("routing.py", kf_prompt)

    def test_verdict_parsers(self):
        self.assertEqual(
            run.parse_verdict('{"correct": true, "score": 1.4, "reason": "ok"}'),
            {"correct": True, "score": 1.0, "reason": "ok"},
        )
        self.assertIsNone(run.parse_verdict("yes"))
        self.assertIsNone(
            run.parse_verdict('{"correct": "yes", "score": 1, "reason": ""}')
        )
        self.assertEqual(run.parse_yes_no("Yes.")["correct"], True)  # ty: ignore[not-subscriptable]
        self.assertEqual(run.parse_yes_no("no")["correct"], False)  # ty: ignore[not-subscriptable]
        self.assertIsNone(run.parse_yes_no("  "))


class Harness(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.old_home = os.environ.get("DIJKSTRAL_BENCH_HOME")
        self.old_ws = os.environ.get("AGENT_BENCH_WS")
        os.environ["DIJKSTRAL_BENCH_HOME"] = str(self.tmp)
        os.environ["AGENT_BENCH_WS"] = str(self.tmp / "ws")
        self.ctx = run.Ctx(
            home=self.tmp,
            codex="/usr/bin/false",
            model="gpt-6.1-sol",
            effort="medium",
            timeout=5,
            retries=2,
            backoff=0,
            jobs=1,
            dry_run=False,
            graphify_venv=self.tmp / "tools/graphify",
            graphify_semantic_venv=self.tmp / "tools/graphify-semantic",
            dijkstral=self.tmp / "bin/dijkstral",
            sessions_dir=self.tmp / "agent/sessions",
            judge_dir=self.tmp / "agent/judge",
        )
        self.ctx.workspace("fastapi", "none").mkdir(parents=True)
        self.ctx.workspace("fastapi", "dijkstral").mkdir(parents=True)
        self.calls = []

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)
        if self.old_home is None:
            os.environ.pop("DIJKSTRAL_BENCH_HOME", None)
        else:
            os.environ["DIJKSTRAL_BENCH_HOME"] = self.old_home
        if self.old_ws is None:
            os.environ.pop("AGENT_BENCH_WS", None)
        else:
            os.environ["AGENT_BENCH_WS"] = self.old_ws

    def fake_runner(self, *fixtures: str):
        """A runner that writes the next fixture as Codex's stdout; exit 0 only for an answer."""
        queue = list(fixtures)

        def runner(cmd, env, cwd, timeout, stdout, stderr):
            self.calls.append(cmd)
            name = queue.pop(0)
            stdout.write_text((FIX / name).read_text())
            stderr.write_text("")
            return (
                0
                if name in ("codex-events-real.jsonl", "codex-events-fake-mcp.jsonl")
                else 1
            ), False

        return runner

    def test_session_command_flags(self):
        t = task()
        none = run.session_command(self.ctx, t, "none", Path("/x/last.txt"))
        self.assertEqual(none[:2], ["/usr/bin/false", "exec"])
        for flag in (
            "--ephemeral",
            "--ignore-user-config",
            "--json",
            "--skip-git-repo-check",
        ):
            self.assertIn(flag, none)
        self.assertEqual(none[none.index("-s") + 1], "read-only")
        self.assertEqual(none[none.index("-m") + 1], "gpt-6.1-sol")
        self.assertIn('model_reasoning_effort="medium"', none)
        self.assertIn('web_search="disabled"', none)
        self.assertEqual(none[none.index("--disable") + 1], "apps")
        self.assertFalse(any("mcp_servers" in a for a in none))
        self.assertNotIn("--dangerously-bypass-hook-trust", none)
        self.assertEqual(none[-1], run.build_prompt(t))
        dij = run.session_command(self.ctx, t, "dijkstral", Path("/x/last.txt"))
        ws = self.ctx.workspace("fastapi", "dijkstral")
        self.assertIn(
            f'mcp_servers.dijkstral.args=["mcp", "{ws}/dijkstral-out/graph.dij"]', dij
        )
        self.assertIn(
            'mcp_servers.dijkstral.default_tools_approval_mode="approve"', dij
        )
        gra = run.session_command(self.ctx, t, "graphify", Path("/x/last.txt"))
        self.assertIn(
            'mcp_servers.graphify.env={GRAPHIFY_QUERY_LOG_DISABLE = "1"}', gra
        )
        self.assertNotIn(
            "dijkstral",
            str(self.ctx.workspace("fastapi", "none").relative_to(self.tmp / "ws")),
        )
        # By default the workspaces sit beside the bench home, outside its tool-named path.
        os.environ.pop("AGENT_BENCH_WS")
        self.assertEqual(run.ws_root(), self.tmp.parent / "codebench" / "ws")
        os.environ["AGENT_BENCH_WS"] = str(self.tmp / "ws")

    def test_answering_judge_and_build_models_are_separate(self):
        import dataclasses

        ctx = dataclasses.replace(self.ctx, model="gpt-6-luna", effort="low")
        answer = run.codex_base(ctx, self.tmp)
        judge = run.codex_base(ctx, self.tmp, judge=True)
        self.assertEqual(answer[answer.index("-m") + 1], "gpt-6-luna")
        self.assertIn('model_reasoning_effort="low"', answer)
        self.assertEqual(judge[judge.index("-m") + 1], run.MODEL)
        self.assertIn(f'model_reasoning_effort="{run.EFFORT}"', judge)
        # The judge cache does not depend on the answering model.
        task = {
            "id": "t",
            "suite": "keyfacts",
            "question": "q",
            "reference": "r",
            "facts": [],
        }
        self.assertEqual(
            run.judge_key(ctx, task, "a"), run.judge_key(self.ctx, task, "a")
        )

    def test_session_env(self):
        os.environ["SOME_API_KEY"] = "x"
        os.environ["HERDR_PANE_ID"] = "1"
        try:
            env = run.session_env(self.ctx, "dijkstral", False)
        finally:
            del os.environ["SOME_API_KEY"], os.environ["HERDR_PANE_ID"]
        self.assertNotIn("SOME_API_KEY", env)
        self.assertNotIn("HERDR_PANE_ID", env)
        self.assertEqual(env["GRAPHIFY_QUERY_LOG_DISABLE"], "1")
        self.assertTrue(
            env["PATH"].startswith(
                str(self.ctx.tool_dir("dijkstral", False)) + os.pathsep
            )
        )
        self.assertFalse(
            run.session_env(self.ctx, "none", False)["PATH"].startswith(str(self.tmp))
        )

    def test_other_graphify_version_gets_its_own_paths(self):
        def tool_dir_name(arm: str, semantic: bool) -> str:
            d = self.ctx.tool_dir(arm, semantic)
            assert d is not None
            return d.name

        self.assertEqual(tool_dir_name("graphify", False), "graphify")
        with mock.patch.object(run, "GRAPHIFY_VERSION", "0.9.97"):
            self.assertEqual(self.ctx.slot("graphify"), "graphify-0.9.97")
            self.assertEqual(tool_dir_name("graphify", False), "graphify-0.9.97")
            self.assertEqual(
                tool_dir_name("graphify", True), "graphify-0.9.97-semantic"
            )
            self.assertEqual(tool_dir_name("dijkstral", False), "dijkstral")

    def test_base_path_drops_tool_dirs(self):
        d = self.tmp / "toolbin"
        d.mkdir()
        (d / "graphify").write_text("#!/bin/sh\n")
        (d / "graphify").chmod(0o755)
        keep, dropped = run.base_path(f"/usr/bin{os.pathsep}{d}")
        self.assertEqual(keep, ["/usr/bin"])
        self.assertEqual(dropped, [str(d)])

    def test_ok_session_writes_result_and_resume_skips(self):
        t = task()
        self.ctx.runner = self.fake_runner("codex-events-real.jsonl")
        self.assertEqual(run.execute_session(self.ctx, t, "none"), "ok")
        paths = run.session_paths(self.ctx, "none", t["id"])
        r = json.loads(paths["result"].read_text())
        self.assertEqual(
            (r["status"], r["answer"], r["usage"]["output_tokens"]),
            ("ok", "PELICAN yes", 126),
        )
        self.assertTrue(paths["events"].exists())
        self.assertEqual(r["command"][-1], "<prompt>")
        # Resume: a stored ok result with the same key is not run again.
        self.assertTrue(run.session_done(self.ctx, t, "none"))
        self.assertEqual(run.execute_session(self.ctx, t, "none"), "skipped")
        self.assertEqual(len(self.calls), 1)
        # A changed prompt (question edited) invalidates the stored result.
        self.assertFalse(
            run.session_done(self.ctx, task(question="Something else?"), "none")
        )
        self.assertFalse(run.session_done(self.ctx, task(), "dijkstral"))

    def test_transient_then_ok_retries(self):
        self.ctx.runner = self.fake_runner(
            "codex-events-transient.jsonl", "codex-events-real.jsonl"
        )
        self.assertEqual(run.execute_session(self.ctx, task(), "none"), "ok")
        r = json.loads(
            run.session_paths(self.ctx, "none", "fastapi-k01")["result"].read_text()
        )
        self.assertEqual([a["status"] for a in r["attempts"]], ["transient", "ok"])

    def test_retries_are_bounded(self):
        self.ctx.runner = self.fake_runner(*["codex-events-transient.jsonl"] * 3)
        self.assertEqual(run.execute_session(self.ctx, task(), "none"), "failed")
        self.assertEqual(len(self.calls), 3)
        paths = run.session_paths(self.ctx, "none", "fastapi-k01")
        self.assertFalse(paths["result"].exists())
        self.assertTrue(paths["failed"].exists())
        self.assertFalse(
            run.session_done(self.ctx, task(), "none")
        )  # retried on the next run

    def test_quota_stops_without_retry_or_result(self):
        self.ctx.runner = self.fake_runner("codex-events-quota.jsonl")
        self.assertEqual(run.execute_session(self.ctx, task(), "none"), "quota")
        self.assertEqual(len(self.calls), 1)
        self.assertTrue(self.ctx.stop.is_set())
        paths = run.session_paths(self.ctx, "none", "fastapi-k01")
        self.assertFalse(paths["result"].exists())
        self.assertFalse(paths["failed"].exists())

    def test_queue_stops_launching_after_quota_and_exits_75(self):
        tasks = [task(id=f"t{i}") for i in range(6)]
        fixtures = ["codex-events-real.jsonl", "codex-events-quota.jsonl"] + [
            "codex-events-real.jsonl"
        ] * 4
        self.ctx.runner = self.fake_runner(*fixtures)
        counts = run.run_queue(
            tasks, lambda t: run.execute_session(self.ctx, t, "none"), 1, self.ctx.stop
        )
        self.assertEqual(counts, {"ok": 1, "quota": 1, "not_started": 4})
        self.assertEqual(len(self.calls), 2)
        with self.assertRaises(SystemExit) as cm:
            run.quota_exit(self.ctx, counts)
        self.assertEqual(cm.exception.code, run.QUOTA_EXIT)
        # The rerun resumes: t0 is skipped, t1 onwards run.
        self.ctx.stop = threading.Event()
        self.ctx.runner = self.fake_runner(*["codex-events-real.jsonl"] * 5)
        counts = run.run_queue(
            tasks, lambda t: run.execute_session(self.ctx, t, "none"), 2, self.ctx.stop
        )
        self.assertEqual(counts, {"skipped": 1, "ok": 5})

    def test_in_flight_sessions_finish_after_quota(self):
        started, release = threading.Event(), threading.Event()
        results = []

        def execute(i):
            if i == 0:
                started.set()
                release.wait(5)
                results.append(i)
                return "ok"
            started.wait(5)
            if i == 1:
                self.ctx.stop.set()
                release.set()
                return "quota"
            results.append(i)
            return "ok"

        counts = run.run_queue(list(range(5)), execute, 2, self.ctx.stop)
        self.assertEqual(counts, {"ok": 1, "quota": 1, "not_started": 3})
        self.assertEqual(results, [0])

    def test_build_quota_detection(self):
        tr = self.tmp / "transcripts"
        tr.mkdir()
        (tr / "req-0001.json").write_text(
            json.dumps({"response": {"ok": True, "errors": []}})
        )
        self.assertFalse(run.build_quota_hit(["built 12 nodes"], tr, None))
        (tr / "req-0002.json").write_text(
            json.dumps(
                {
                    "response": {
                        "ok": False,
                        "errors": ["You've hit your usage limit. Try again later."],
                    }
                }
            )
        )
        self.assertTrue(run.build_quota_hit([], tr, None))
        ev = self.tmp / "calls.events.jsonl"
        shutil.copy(FIX / "codex-events-quota.jsonl", ev)
        self.assertTrue(run.build_quota_hit([], None, ev))

    def test_graded_answer_strips_workspace_prefix(self):
        ws = str(self.ctx.workspace("kubernetes", "graphify"))
        r = {
            "answer": f"See [schedule_one.go]({ws}/pkg/scheduler/schedule_one.go:120).",
            "workspace": ws,
        }
        a = run.graded_answer(r)
        self.assertEqual(a, "See [schedule_one.go](pkg/scheduler/schedule_one.go:120).")
        self.assertTrue(
            score.fact_matches({"fact": "pkg/scheduler/schedule_one.go"}, a)
        )
        self.assertFalse(
            score.fact_matches({"fact": "pkg/scheduler/schedule_one.go"}, r["answer"])
        )

    def test_load_tasks_validates(self):
        f = self.tmp / "t.jsonl"
        f.write_text(
            json.dumps(task()) + "\n" + json.dumps(task(id="b", semantic=True)) + "\n"
        )
        with self.assertRaises(SystemExit):
            run.load_tasks(f)  # one corpus_id, two semantic values
        f.write_text(json.dumps(task(corpus="~/x")) + "\n")
        self.assertEqual(run.load_tasks(f)[0]["corpus"], str(Path.home() / "x"))

    def test_rm_tree_refuses_outside_agent_dir(self):
        with self.assertRaises(SystemExit):
            run.rm_tree(Path.home() / ".cache/dijkstral-bench/corpus/fastapi")


class CommittedTaskFiles(unittest.TestCase):
    def test_memory_task_files_load(self):
        for name in ("locomo.jsonl", "longmemeval.jsonl"):
            path = HERE / "tasks" / name
            if path.exists():
                tasks = run.load_tasks(path)
                self.assertTrue(tasks)
                for t in tasks[:20]:
                    p = run.build_prompt(t)
                    self.assertNotIn(str(t.get("source_id")), p)
                    if t.get("distractor"):
                        self.assertNotIn(t["distractor"], p)
                        self.assertNotIn(t["distractor"], run.judge_spec(t, "x")[0])


if __name__ == "__main__":
    unittest.main()
