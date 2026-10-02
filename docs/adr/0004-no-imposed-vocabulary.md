# The tool imposes no house vocabulary

The names and shapes of a team's recurring status messages are learned from the
description its users give at setup, and stored per Job. The code contains no canonical
names for them.

## Why

Teams differ, and this tool is not for one team. A hard-coded vocabulary is correct in
exactly one workspace and misleading everywhere else. Worse, it fails quietly: the tool
would keep working, keep producing well-formed posts, and keep calling a team's "Day
Status" something the team never asked for.

Making the vocabulary user-supplied also puts it in one place. The Job already holds a
prompt template, a channel, and a window; the message name and shape belong beside them,
not in a constant.

## Consequences

There is no built-in default for what a team's posts look like. Someone setting the tool
up describes their practice in their own words, and the Plan Role turns that into Jobs.

This rules out shipping our own conventions as a starting point, which is tempting and
wrong. Offering them as *editable* defaults is still wrong: it biases every user toward
one team's practice, and the bias is invisible because the defaults look like a starting
point rather than an opinion.

The cost is real. With no defaults, the first Fire depends entirely on how well the user
described their routine. A vague description produces a vague Draft, and the honest
response to that is for the Draft to be visibly thin rather than for the tool to guess at
a convention it was never told about.