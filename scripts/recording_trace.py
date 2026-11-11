"""Correlate actual Display publications, private encoder admissions and packets."""
import re

def correlate(text, session, task, saved_frames):
    prefix=rf'session={session} task={task} '
    published=list(re.finditer(r'\[display.recording.frame\] '+prefix+r'generation=(\d+) publication_ms=(\d+)\b',text))
    admitted=list(re.finditer(r'\[record.frame.accepted\] '+prefix+r'index=(\d+) generation=(\d+) publication_ms=(\d+) pts_ms=(\d+)\r?\n',text))
    assert 1<=saved_frames<=301 and len(admitted)==saved_frames,'incomplete bounded encoder receipts'
    by_generation={}
    previous=None
    for match in published:
        generation,stamp=map(int,match.groups())
        assert generation not in by_generation,'duplicate Display publication generation'
        if previous: assert generation>previous[0] and stamp>previous[1],'unordered Display publications'
        previous=generation,stamp;by_generation[generation]=(stamp,match.start())
    rows=[];origin=None;last_generation=None
    for expected,match in enumerate(admitted,1):
        index,generation,stamp,pts=map(int,match.groups())
        assert index==expected and generation in by_generation,'missing/reordered admission or unknown generation'
        observed,position=by_generation[generation]
        assert observed==stamp and position<match.start(),'admission differs from actual preceding Display publication'
        if last_generation is not None:assert generation>last_generation,'duplicate/out-of-order encoder generation'
        last_generation=generation
        if origin is None:origin=stamp
        assert pts==stamp-origin,'encoder timestamp does not preserve publication-time delta'
        rows.append(dict(index=index,generation=generation,publication_ms=stamp,pts_ms=pts))
    encoded={row['generation']for row in rows}
    return dict(encoded_frames=rows,dropped_publications=[dict(generation=g,publication_ms=v[0])for g,v in by_generation.items()if g not in encoded])

def check_packets(trace, packet_pts_ms):
    assert packet_pts_ms==[row['pts_ms']for row in trace['encoded_frames']], 'decoded packet PTS differs from actual Display publication deltas'
