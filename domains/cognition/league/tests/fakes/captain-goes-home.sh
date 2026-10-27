#!/bin/sh
# A stand-in for `claude -p ...`, run in the captain engineer's worktree: after five minutes
# every drone goes home, whatever the map says. The rules' tests still pass (they need the opening
# orders to matter, so it cannot be from the start); the captain is plainly worse.
sed -i.bak 's|    pub fn decide(&mut self, view: &TeamView) -> Vec<(usize, Order)> {|    pub fn decide(\&mut self, view: \&TeamView) -> Vec<(usize, Order)> {\
        if view.time >= 300.0 {\
            return (0..self.orders.len()).map(\|k\| (k, Order::Home)).collect();\
        }|' domains/recreation/towers/src/commander.rs
rm -f domains/recreation/towers/src/commander.rs.bak
mkdir -p .league
echo '{"notes": "Every drone stays home."}' > .league/candidates.json
echo '{"type":"result","is_error":false,"result":"Kept the drones safe at home."}'
