# Edit Parser

The Edit Parser is a component of the OpenAI Codex project that handles parsing of file edits and tool calls in the response. It allows for editing files, creating new ones, deleting existing ones, and executing commands. Here's a brief guide on how to use it:

## File Edits
To edit a file, you need to indicate the path to the file, provide old contents (optional), and specify new content with the following syntax:

<<<EDIT path/to/file.rs
--- OLD
exact lines to replace
--- NEW
replacement lines