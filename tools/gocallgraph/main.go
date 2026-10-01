// Command gocallgraph writes the call graph of a Go module as JSON lines, one
// edge per line, keyed by call site (codetags PLAN.md §9 P1.5, brief §4.4 Go
// row).
//
// Usage:
//
//	gocallgraph [-C DIR] [-o FILE] [-algo auto|vta|cha] [PACKAGE PATTERN...]
//
// It loads the patterns, ./... by default, in the module root DIR (the
// current directory by default); -o is relative to the current directory. With -algo auto it uses VTA when the program has a main
// package and CHA otherwise, since VTA only follows values that flow from
// code it can see, and a library without main has no such code.
//
// An edge's caller is a function of a loaded package; calls inside stdlib
// and dependency code are not written. Polymorphic calls write one edge per
// callee at the same site. Positions are 1-based lines and 1-based byte
// columns; the call's column is the start of the called name (the selector
// of x.f(), the identifier of f()), where a SCIP indexer puts its reference.
//
// Any package error, a pattern that matches nothing, or an output error
// exits 1 with the reason on stderr.
package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"golang.org/x/tools/go/callgraph"
	"golang.org/x/tools/go/callgraph/cha"
	"golang.org/x/tools/go/callgraph/vta"
	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
	"golang.org/x/tools/go/ssa/ssautil"
)

// Edge is one line of output.
type Edge struct {
	// Caller is the calling function, as go/ssa names it, e.g.
	// "example.com/shop/internal/pay.Settle" or "example.com/shop/cmd/shop.main$1"
	// for a function literal. A generic function's instances are written as
	// the generic function.
	Caller string `json:"caller"`
	// Callee is the called function, named the same way, e.g.
	// "(example.com/shop/internal/pay.Card).Charge".
	Callee string `json:"callee"`
	// File is the call site's file, relative to the module root, with '/'.
	File string `json:"file"`
	// Line is the call site's 1-based line.
	Line int `json:"line"`
	// Column is the call site's 1-based byte column.
	Column int `json:"column"`
	// Kind is "static" when the call names its callee, else "dynamic"
	// (interface methods, function values).
	Kind string `json:"kind"`
	// Algorithm is "vta" or "cha".
	Algorithm string `json:"algorithm"`
	// CalleeFile, CalleeLine and CalleeColumn place the callee's name when
	// it is declared in the module; they are omitted otherwise.
	CalleeFile   string `json:"callee_file,omitempty"`
	CalleeLine   int    `json:"callee_line,omitempty"`
	CalleeColumn int    `json:"callee_column,omitempty"`
}

func main() {
	if err := run(os.Args[1:], os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, "gocallgraph:", err)
		os.Exit(1)
	}
}

func run(args []string, stdout io.Writer) error {
	flags := flag.NewFlagSet("gocallgraph", flag.ContinueOnError)
	dir := flags.String("C", ".", "module root")
	output := flags.String("o", "-", "output file, or - for stdout")
	algo := flags.String("algo", "auto", "auto, vta, or cha")
	if err := flags.Parse(args); err != nil {
		return err
	}
	switch *algo {
	case "auto", "vta", "cha":
	default:
		return fmt.Errorf("unknown -algo %q", *algo)
	}
	patterns := flags.Args()
	if len(patterns) == 0 {
		patterns = []string{"./..."}
	}
	root, err := filepath.Abs(*dir)
	if err != nil {
		return err
	}
	// Compare real paths: on macOS a temporary root under /var is
	// /private/var once resolved, and go list may report either.
	if root, err = filepath.EvalSymlinks(root); err != nil {
		return err
	}

	edges, err := callGraph(root, patterns, *algo)
	if err != nil {
		return err
	}

	out := stdout
	if *output != "-" {
		file, err := os.Create(*output)
		if err != nil {
			return err
		}
		defer file.Close()
		out = file
	}
	writer := bufio.NewWriter(out)
	encoder := json.NewEncoder(writer)
	for _, edge := range edges {
		if err := encoder.Encode(edge); err != nil {
			return err
		}
	}
	return writer.Flush()
}

func callGraph(root string, patterns []string, algo string) ([]Edge, error) {
	config := &packages.Config{Mode: packages.LoadAllSyntax, Dir: root}
	pkgs, err := packages.Load(config, patterns...)
	if err != nil {
		return nil, err
	}
	if len(pkgs) == 0 {
		return nil, fmt.Errorf("no packages match %v", patterns)
	}
	if n := packages.PrintErrors(pkgs); n > 0 {
		return nil, fmt.Errorf("%d package errors", n)
	}

	prog, ssaPkgs := ssautil.AllPackages(pkgs, ssa.InstantiateGenerics)
	prog.Build()

	if algo == "auto" {
		algo = "cha"
		if len(ssautil.MainPackages(ssaPkgs)) > 0 {
			algo = "vta"
		}
	}
	var graph *callgraph.Graph
	switch algo {
	case "vta":
		graph = vta.CallGraph(ssautil.AllFunctions(prog), cha.CallGraph(prog))
	default:
		graph = cha.CallGraph(prog)
	}

	loaded := map[*ssa.Package]bool{}
	for _, pkg := range ssaPkgs {
		if pkg != nil {
			loaded[pkg] = true
		}
	}
	sites := callExprs(pkgs)
	edges := map[Edge]bool{}
	for fn, node := range graph.Nodes {
		if fn == nil || !loaded[origin(fn).Pkg] {
			continue
		}
		for _, out := range node.Out {
			edge, ok := makeEdge(prog.Fset, root, sites, out, algo)
			if ok {
				edges[edge] = true
			}
		}
	}
	if len(edges) == 0 && hasCalls(pkgs) {
		return nil, errors.New("no call edges, but the loaded packages make calls")
	}

	sorted := make([]Edge, 0, len(edges))
	for edge := range edges {
		sorted = append(sorted, edge)
	}
	sort.Slice(sorted, func(i, j int) bool { return less(sorted[i], sorted[j]) })
	return sorted, nil
}

// origin is the generic function an instance was made from, or fn itself.
func origin(fn *ssa.Function) *ssa.Function {
	if o := fn.Origin(); o != nil {
		return o
	}
	return fn
}

// name names fn for output, as its origin when it is an instance.
func name(fn *ssa.Function) string {
	return origin(fn).String()
}

// makeEdge turns a call-graph edge into an output edge. It drops edges with
// no position in the module: calls inside synthetic wrappers.
func makeEdge(fset *token.FileSet, root string, sites map[token.Pos]*ast.CallExpr, out *callgraph.Edge, algo string) (Edge, bool) {
	if out.Site == nil || out.Callee.Func == nil {
		return Edge{}, false
	}
	pos := out.Site.Pos()
	if call, ok := sites[out.Site.Common().Pos()]; ok {
		pos = calleePos(call.Fun)
	}
	file, line, column, ok := place(fset, root, pos)
	if !ok {
		return Edge{}, false
	}
	kind := "dynamic"
	if out.Site.Common().StaticCallee() != nil {
		kind = "static"
	}
	callee := out.Callee.Func
	edge := Edge{
		Caller:    name(out.Caller.Func),
		Callee:    name(callee),
		File:      file,
		Line:      line,
		Column:    column,
		Kind:      kind,
		Algorithm: algo,
	}
	if target := declared(callee); target != nil {
		callee = target
		edge.Callee = name(target)
	}
	if file, line, column, ok := place(fset, root, origin(callee).Pos()); ok {
		edge.CalleeFile, edge.CalleeLine, edge.CalleeColumn = file, line, column
	}
	return edge, true
}

// declared is the declared method that a synthetic wrapper stands for, or
// nil when fn is not a wrapper. go/ssa makes wrappers for promoted and
// pointer-receiver methods ("wrapper for"), method expressions ("thunk
// for"), and method values ("bound method wrapper for": the callee of f()
// after f := x.M). An edge to a wrapper is written as an edge to the method.
func declared(fn *ssa.Function) *ssa.Function {
	wrapper := false
	for _, prefix := range []string{"wrapper for ", "thunk for ", "bound method wrapper for "} {
		wrapper = wrapper || strings.HasPrefix(fn.Synthetic, prefix)
	}
	obj, ok := fn.Object().(*types.Func)
	if !wrapper || !ok {
		return nil
	}
	return fn.Prog.FuncValue(obj)
}

// calleePos is where the called name starts in a call's Fun expression.
func calleePos(fun ast.Expr) token.Pos {
	switch fun := fun.(type) {
	case *ast.SelectorExpr:
		return fun.Sel.Pos()
	case *ast.ParenExpr:
		return calleePos(fun.X)
	case *ast.IndexExpr:
		return calleePos(fun.X)
	case *ast.IndexListExpr:
		return calleePos(fun.X)
	default:
		return fun.Pos()
	}
}

// place is pos as a module-relative file with 1-based line and column, or
// false when pos is unknown or outside the module.
func place(fset *token.FileSet, root string, pos token.Pos) (string, int, int, bool) {
	if !pos.IsValid() {
		return "", 0, 0, false
	}
	position := fset.Position(pos)
	filename := position.Filename
	if real, err := filepath.EvalSymlinks(filename); err == nil {
		filename = real
	}
	rel, err := filepath.Rel(root, filename)
	if err != nil || !filepath.IsLocal(rel) {
		return "", 0, 0, false
	}
	return filepath.ToSlash(rel), position.Line, position.Column, true
}

// callExprs maps each call's Lparen, the position go/ssa gives a call, to
// the call, over the loaded packages' syntax.
func callExprs(pkgs []*packages.Package) map[token.Pos]*ast.CallExpr {
	calls := map[token.Pos]*ast.CallExpr{}
	for _, pkg := range pkgs {
		for _, file := range pkg.Syntax {
			ast.Inspect(file, func(node ast.Node) bool {
				if call, ok := node.(*ast.CallExpr); ok {
					calls[call.Lparen] = call
				}
				return true
			})
		}
	}
	return calls
}

// hasCalls says whether the loaded packages' syntax holds a call that is
// not a conversion or a builtin, so that an empty graph is an error.
func hasCalls(pkgs []*packages.Package) bool {
	for _, pkg := range pkgs {
		for _, file := range pkg.Syntax {
			found := false
			ast.Inspect(file, func(node ast.Node) bool {
				call, ok := node.(*ast.CallExpr)
				if ok && pkg.TypesInfo.Types[call.Fun].IsValue() {
					found = true
				}
				return !found
			})
			if found {
				return true
			}
		}
	}
	return false
}

func less(a, b Edge) bool {
	if a.File != b.File {
		return a.File < b.File
	}
	if a.Line != b.Line {
		return a.Line < b.Line
	}
	if a.Column != b.Column {
		return a.Column < b.Column
	}
	if a.Caller != b.Caller {
		return a.Caller < b.Caller
	}
	return a.Callee < b.Callee
}
