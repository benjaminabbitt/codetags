Feature: Windows names map reserved characters to private-use code points
  Win32 forbids `" * : < > ? |` in a file name. Cygwin, MSYS2 (Git Bash) and
  WSL store each as U+F000 plus its ASCII code (PLAN.md §2.7, D15, V39). The
  WinFsp backend and the Windows static-tree writer use the same mapping, so
  agents in Git Bash or WSL type and see the real characters.

  Here `{U+F03A}` stands for the one code point U+F03A.

  Scenario Outline: Reserved characters map to private-use code points and back
    When the name "<name>" is mapped for Windows
    Then the Windows name is "<windows>"
    And the Windows name maps back to "<name>"

    Examples:
      | name                    | windows                                        |
      | kind=function           | kind=function                                  |
      | prov:source=scip        | prov{U+F03A}source=scip                        |
      | line>50                 | line{U+F03E}50                                 |
      | line<50                 | line{U+F03C}50                                 |
      | owner=*                 | owner={U+F02A}                                 |
      | a?b\|c                  | a{U+F03F}b{U+F07C}c                            |
      | name~billing.Charge<T>  | name~billing.Charge{U+F03C}T{U+F03E}           |

  Scenario: A quoted value keeps its quotes through the mapping
    When the name 'calls="billing.Charge<T>.apply"' is mapped for Windows
    Then the Windows name is "calls={U+F022}billing.Charge{U+F03C}T{U+F03E}.apply{U+F022}"
    And the Windows name maps back to 'calls="billing.Charge<T>.apply"'
