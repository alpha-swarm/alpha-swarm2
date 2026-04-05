# Feature: Parse LLM Edit Blocks

## Scenario: Parse single EDIT block
  Given response contains <<<EDIT src/main.rs with OLD/NEW markers
  When I parse the response
  Then I get 1 FileEdit::Edit with correct path, old, and new content

## Scenario: Parse CREATE block
  Given response contains <<<CREATE src/new.rs with file content
  When I parse the response
  Then I get 1 FileEdit::Create with path and content

## Scenario: Parse DELETE block
  Given response contains <<<DELETE src/old.rs
  When I parse the response
  Then I get 1 FileEdit::Delete with the path

## Scenario: Parse multiple mixed blocks
  Given response with EDIT, CREATE, and DELETE blocks
  When I parse the response
  Then I get 3 FileEdits in order

## Scenario: Unclosed block returns error
  Given response with <<<EDIT but no >>>
  When I parse the response
  Then I get a ParseError mentioning "Unclosed"

## Scenario: Missing OLD marker returns error
  Given an EDIT block without --- OLD
  When I parse the response
  Then I get a ParseError mentioning "missing --- OLD"

## Scenario: Empty response returns empty list
  Given an empty string
  When I parse the response
  Then I get an empty Vec

## Scenario: Text around blocks is ignored
  Given response with explanation text before and after edit blocks
  When I parse the response
  Then only the edit blocks are extracted

## Scenario: Blocks with code containing angle brackets
  Given an EDIT block where the code contains Vec<String> or Result<T>
  When I parse the response
  Then it correctly finds >>> as the block delimiter (not <> in code)
