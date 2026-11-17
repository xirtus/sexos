# Qupid Q3: Up next

Each playlist item knows what plays after it: the next item, or at the playlist's end the
show's next episode from the same folder (`sex_episodes::episodes::next_episode` over the
listing, when Qupid may read the folder). `UpNextCard` shows "Up next S01E02 in N s" from
the credits chapter or 30 s before the end, counts 10 s of playing, then the next item
starts; Enter plays now, Esc dismisses until a seek back. Gate
`scripts/qupid_upnext_gate.py` (CI media job): three 60 s episodes; auto-advance, Esc,
and no card in the last one.
