import asyncio
import gc
import os
import tempfile

import cfmd_async_probe


def inserted(event):
    source, target, added, removed = event
    assert target > source
    assert removed == []
    return added


def todo_added(event):
    source, target, added, removed = event
    assert target > source
    return added, removed


async def await_next_roundtrip(db):
    watch = db.watch("left")
    pending = asyncio.ensure_future(watch.next())
    await asyncio.sleep(0)
    db.insert("left", 11)
    assert inserted(await asyncio.wait_for(pending, 2.0)) == [11]


async def async_for_roundtrip(db):
    watch = db.watch("right")
    db.insert("right", 21)
    async for event in watch:
        assert inserted(event) == [21]
        break


async def cancellation_preserves_durable_event(db):
    watch = db.watch("left")
    abandoned = asyncio.ensure_future(watch.next())
    await asyncio.sleep(0)
    abandoned.cancel()
    try:
        await abandoned
    except asyncio.CancelledError:
        pass
    else:
        raise AssertionError("cancelled Python task unexpectedly completed")
    gc.collect()

    db.insert("left", 31)
    assert inserted(await asyncio.wait_for(watch.next(), 2.0)) == [31]


async def dependency_frontier_suppresses_unrelated_wake(db):
    left = db.watch("left")
    right = db.watch("right")
    left_pending = asyncio.ensure_future(left.next())
    right_pending = asyncio.ensure_future(right.next())
    await asyncio.sleep(0)

    db.insert("left", 41)
    assert inserted(await asyncio.wait_for(left_pending, 2.0)) == [41]
    await asyncio.sleep(0.05)
    assert not right_pending.done(), "unrelated relation woke the Python asyncio watch"

    right_pending.cancel()
    try:
        await right_pending
    except asyncio.CancelledError:
        pass
    db.insert("right", 42)
    assert inserted(await asyncio.wait_for(right.next(), 2.0)) == [42]


async def durable_backlog_is_drained_exactly(db):
    watch = db.watch("left")
    for value in (51, 52, 53, 54):
        db.insert("left", value)
    for value in (51, 52, 53, 54):
        assert inserted(await asyncio.wait_for(watch.next(), 2.0)) == [value]


async def repeated_cancel_rearm(db):
    watch = db.watch("right")
    for _ in range(64):
        pending = asyncio.ensure_future(watch.next())
        await asyncio.sleep(0)
        pending.cancel()
        try:
            await pending
        except asyncio.CancelledError:
            pass
    gc.collect()
    db.insert("right", 61)
    assert inserted(await asyncio.wait_for(watch.next(), 2.0)) == [61]


async def cancellation_commit_race_is_lossless(db):
    watch = db.watch("left")
    for value in range(1000, 1064):
        pending = asyncio.ensure_future(watch.next())
        await asyncio.sleep(0)
        pending.cancel()
        db.insert("left", value)
        try:
            await pending
        except asyncio.CancelledError:
            pass
        assert inserted(await asyncio.wait_for(watch.next(), 2.0)) == [value]


async def queued_next_survives_head_cancellation(db):
    watch = db.watch("right")
    head = asyncio.ensure_future(watch.next())
    await asyncio.sleep(0)
    queued = asyncio.ensure_future(watch.next())
    await asyncio.sleep(0)
    head.cancel()
    try:
        await head
    except asyncio.CancelledError:
        pass
    db.insert("right", 71)
    assert inserted(await asyncio.wait_for(queued, 2.0)) == [71]


async def concurrent_next_is_ordered(db):
    watch = db.watch("left")
    first = asyncio.ensure_future(watch.next())
    second = asyncio.ensure_future(watch.next())
    await asyncio.sleep(0)
    db.insert("left", 81)
    db.insert("left", 82)
    first_event, second_event = await asyncio.wait_for(
        asyncio.gather(first, second), 3.0
    )
    assert inserted(first_event) == [81]
    assert inserted(second_event) == [82]


async def many_pending_watches_receive_exactly_once(db):
    watches = [db.watch("left") for _ in range(512)]
    pending = [asyncio.ensure_future(watch.next()) for watch in watches]
    await asyncio.sleep(0)
    db.insert("left", 91)
    events = await asyncio.wait_for(asyncio.gather(*pending), 6.0)
    assert len(events) == 512
    assert all(inserted(event) == [91] for event in events)


async def many_unrelated_watches_stay_pending(db):
    watches = [db.watch("right") for _ in range(256)]
    pending = [asyncio.ensure_future(watch.next()) for watch in watches]
    await asyncio.sleep(0)
    db.insert("left", 92)
    await asyncio.sleep(0.05)
    assert not any(task.done() for task in pending)
    for task in pending:
        task.cancel()
    await asyncio.gather(*pending, return_exceptions=True)


def product_crud_relations_history(db):
    base_history = db.history_len()

    db.todo_insert(1, "draft", False)
    assert db.todo_get(1) == (1, "draft", False)
    assert (1, "draft", False) in db.todo_query_done(False)

    db.todo_update_title(1, "ship")
    assert db.todo_get(1) == (1, "ship", False)

    db.todo_insert(2, "undo-me", True)
    assert db.todo_get(2) == (2, "undo-me", True)
    assert db.history_len() >= base_history + 3
    db.undo_latest()
    assert db.todo_get(2) is None

    db.user_insert(10, "Alice")
    db.user_insert(11, "Bob")
    db.task_insert(20, "typed-navigation", 10)
    db.task_insert(21, "other", 11)
    assert db.task_titles_for_owner("Alice") == ["typed-navigation"]
    assert db.task_titles_for_owner("Nobody") == []

    db.owner_insert_with_asset(30, "source", 40, "ephemeral")
    db.owner_insert_empty(31, "target")
    assert db.owner_asset_ids(30) == [40]
    assert db.owner_asset_ids(31) == []
    db.owner_move_asset(30, 31, 40)
    assert db.owner_asset_ids(30) == []
    assert db.owner_asset_ids(31) == [40]
    orphaned, normalized, _ = db.owner_detach_label_preview(31, "ephemeral")
    assert orphaned == 1
    assert normalized >= 1
    assert not db.asset_exists(40)

    db.todo_delete(1)
    assert db.todo_get(1) is None


async def typed_object_watch_tracks_membership(db):
    watch = db.todo_watch(False)
    initial_ids = {row[0] for row in watch.initial()}
    assert 100 not in initial_ids

    db.todo_insert(100, "observable", False)
    added, removed = todo_added(await asyncio.wait_for(watch.next(), 2.0))
    assert added == [(100, "observable", False)]
    assert removed == []

    db.todo_set_done(100, True)
    added, removed = todo_added(await asyncio.wait_for(watch.next(), 2.0))
    assert added == []
    assert removed == [(100, "observable", False)]

    db.todo_set_done(100, False)
    added, removed = todo_added(await asyncio.wait_for(watch.next(), 2.0))
    assert added == [(100, "observable", False)]
    assert removed == []


async def typed_watch_cancellation_is_lossless(db):
    watch = db.todo_watch(False)
    pending = asyncio.ensure_future(watch.next())
    await asyncio.sleep(0)
    pending.cancel()
    db.todo_insert(101, "cancel-race", False)
    try:
        await pending
    except asyncio.CancelledError:
        pass
    added, removed = todo_added(await asyncio.wait_for(watch.next(), 2.0))
    assert added == [(101, "cancel-race", False)]
    assert removed == []


async def gc_churn_does_not_poison_new_consumers(db):
    for _ in range(128):
        watch = db.watch("right")
        pending = asyncio.ensure_future(watch.next())
        await asyncio.sleep(0)
        pending.cancel()
        await asyncio.gather(pending, return_exceptions=True)
        del pending
        del watch
    gc.collect()

    fresh = db.watch("right")
    db.insert("right", 111)
    assert inserted(await asyncio.wait_for(fresh.next(), 2.0)) == [111]


async def main(db):
    product_crud_relations_history(db)
    await await_next_roundtrip(db)
    await async_for_roundtrip(db)
    await cancellation_preserves_durable_event(db)
    await dependency_frontier_suppresses_unrelated_wake(db)
    await durable_backlog_is_drained_exactly(db)
    await repeated_cancel_rearm(db)
    await cancellation_commit_race_is_lossless(db)
    await queued_next_survives_head_cancellation(db)
    await concurrent_next_is_ordered(db)
    await many_pending_watches_receive_exactly_once(db)
    await many_unrelated_watches_stay_pending(db)
    await typed_object_watch_tracks_membership(db)
    await typed_watch_cancellation_is_lossless(db)
    await gc_churn_does_not_poison_new_consumers(db)


def loop_shutdown_releases_pending(db):
    watch = db.watch("left")

    async def abandon():
        asyncio.ensure_future(watch.next())
        await asyncio.sleep(0)

    asyncio.run(abandon())
    gc.collect()
    db.insert("left", 121)

    async def consume():
        assert inserted(await asyncio.wait_for(watch.next(), 2.0)) == [121]

    asyncio.run(consume())


def persistence_roundtrip(root):
    path = os.path.join(root, "persist.cfmd")
    db = cfmd_async_probe.Database(path)
    db.todo_insert(500, "persisted", False)
    assert db.todo_get(500) == (500, "persisted", False)
    del db
    gc.collect()
    assert cfmd_async_probe.Database.persisted_todo(path, 500) == (
        500,
        "persisted",
        False,
    )


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="cfmd-python-hostile-") as directory:
        path = os.path.join(directory, "main.cfmd")
        database = cfmd_async_probe.Database(path)
        asyncio.run(main(database))
        loop_shutdown_releases_pending(database)
        del database
        gc.collect()
        persistence_roundtrip(directory)
    print("CFMD Python hostile/product probe: PASS")
