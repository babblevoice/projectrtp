# -O3 includes -fopt-info-vec -fopt-info-vec-missed
DEBUG:= -DDEBUG -g
RELEASE:=-O3
CCOPTS:=-Wall -fstack-protector-all -std=c++20 -fconcepts -fsanitize=address,leak -DTESTSUITE
LL:=-lboost_system -lpthread -lrt -L./build/Debug/test/ -lspandsp -lilbc -lgnutls -lsrtp2 -lm -fsanitize=address,leak

COMPILER:=g++
LINKER:=g++

CPUCOUNT=$(shell grep -c   "^processor" /proc/cpuinfo)

ifeq (, $(shell which ccache))
$(error "No ccache in $(PATH), use dnf install ccache or apt-get install ccache")
endif

# Some speed options
## 1. Use ccache (note above warning if it is not avaiable)
CCACHE_EXISTS := $(shell ccache -V)
COMPILER:=ccache $(COMPILER)
LINKER:=ccache $(LINKER)


## 2. Have a multi flag to re-run with multi jobs
.PHONY: multi
multi:
	$(MAKE) -j$(CPUCOUNT) default

debug: CCOPTS += $(DEBUG)
debug: ./build/Debug/test/projectrtp

default: debug

# Find all sources
CPPSOURCES:=$(shell ls *.cpp)

# Make targets out of them
ALLOBJS=$(CPPSOURCES:%.cpp=./build/Debug/test/%.o)

./build/Debug/test/projectrtp: outdir $(ALLOBJS)
	$(LINKER) $(ALLOBJS) -o ./build/Debug/test/projectrtp $(LL)

clean:
	rm ./build/Debug/test/ -fr
	ccache -C

outdir:
	mkdir -p ./build/Debug/test/

./build/Debug/test/%.o: %.cpp
	$(COMPILER) -c $(CCOPTS) $(INCLUDE) $(DEFINE) $< -o $@
