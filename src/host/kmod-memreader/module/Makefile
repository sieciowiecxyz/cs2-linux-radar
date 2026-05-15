KDIR ?= /lib/modules/$(shell uname -r)/build
PWD := $(shell pwd)
RUSTC ?= $(shell command -v /usr/bin/rustc 2>/dev/null || command -v rustc)

.PHONY: all clean modules

all: modules

modules:
	$(MAKE) -C $(KDIR) M=$(PWD) RUSTC=$(RUSTC) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) RUSTC=$(RUSTC) clean
