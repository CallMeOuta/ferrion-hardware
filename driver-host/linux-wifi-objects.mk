# SPDX-License-Identifier: GPL-2.0-only
LINUX_VERSION := 6.18.64
LINUX_KBUILD_CFLAGS := -nostdinc -D__KERNEL__ -std=gnu11 -fshort-wchar -funsigned-char -fno-common -fno-PIE -fno-strict-aliasing -mno-sse -mno-mmx -mno-sse2 -mno-3dnow -mno-avx -mno-sse4a -fcf-protection=none -m64 -falign-jumps=1 -falign-loops=1 -mno-80387 -mno-fp-ret-in-387 -mpreferred-stack-boundary=3 -mskip-rax-setup -march=x86-64 -mtune=generic -mno-red-zone -mcmodel=kernel -Wno-sign-compare -fno-asynchronous-unwind-tables -fno-delete-null-pointer-checks -O2 -fno-allow-store-data-races -fno-stack-protector -fomit-frame-pointer -ftrivial-auto-var-init=zero -fno-stack-clash-protection -falign-functions=16 -fstrict-flex-arrays=3 -fno-strict-overflow -fno-stack-check -fconserve-stack -fno-builtin-wcslen -Wall -Wextra -Wundef -Werror=implicit-function-declaration -Werror=implicit-int -Werror=return-type -Werror=strict-prototypes -Wno-format-security -Wno-trigraphs -Wno-frame-address -Wno-address-of-packed-member -Wmissing-declarations -Wmissing-prototypes -Wframe-larger-than=2048 -Wno-main -Wno-dangling-pointer -Wvla-larger-than=1 -Wno-pointer-sign -Wcast-function-type -Wno-array-bounds -Wno-stringop-overflow -Wno-alloc-size-larger-than -Wimplicit-fallthrough=5 -Werror=date-time -Werror=incompatible-pointer-types -Werror=designated-init -Wenum-conversion -Wunused -Wno-unused-but-set-variable -Wno-unused-const-variable -Wno-packed-not-aligned -Wno-format-overflow -Wno-format-truncation -Wno-stringop-truncation -Wno-override-init -Wno-missing-field-initializers -Wno-type-limits -Wno-shift-negative-value -Wno-maybe-uninitialized -Wno-sign-compare -Wno-unused-parameter

LINUX_SOURCES += crypto/acompress
crypto/acompress_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto_acompress"' '-DKBUILD_BASENAME="acompress"' '-DKBUILD_MODNAME="crypto_acompress"' -D__KBUILD_MODNAME=kmod_crypto_acompress
LINUX_SOURCES += crypto/aead
crypto/aead_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/aead"' '-DKBUILD_BASENAME="aead"' '-DKBUILD_MODNAME="aead"' -D__KBUILD_MODNAME=kmod_aead
LINUX_SOURCES += crypto/aes_generic
crypto/aes_generic_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/aes_generic"' '-DKBUILD_BASENAME="aes_generic"' '-DKBUILD_MODNAME="aes_generic"' -D__KBUILD_MODNAME=kmod_aes_generic -fno-code-hoisting
LINUX_SOURCES += crypto/ahash
crypto/ahash_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto_hash"' '-DKBUILD_BASENAME="ahash"' '-DKBUILD_MODNAME="crypto_hash"' -D__KBUILD_MODNAME=kmod_crypto_hash
LINUX_SOURCES += crypto/akcipher
crypto/akcipher_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/akcipher"' '-DKBUILD_BASENAME="akcipher"' '-DKBUILD_MODNAME="akcipher"' -D__KBUILD_MODNAME=kmod_akcipher
LINUX_SOURCES += crypto/algapi
crypto/algapi_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto_algapi"' '-DKBUILD_BASENAME="algapi"' '-DKBUILD_MODNAME="crypto_algapi"' -D__KBUILD_MODNAME=kmod_crypto_algapi
LINUX_SOURCES += crypto/algboss
crypto/algboss_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/cryptomgr"' '-DKBUILD_BASENAME="algboss"' '-DKBUILD_MODNAME="cryptomgr"' -D__KBUILD_MODNAME=kmod_cryptomgr
LINUX_SOURCES += crypto/api
crypto/api_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto"' '-DKBUILD_BASENAME="api"' '-DKBUILD_MODNAME="crypto"' -D__KBUILD_MODNAME=kmod_crypto
LINUX_SOURCES += crypto/ccm
crypto/ccm_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/ccm"' '-DKBUILD_BASENAME="ccm"' '-DKBUILD_MODNAME="ccm"' -D__KBUILD_MODNAME=kmod_ccm
LINUX_SOURCES += crypto/cipher
crypto/cipher_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto"' '-DKBUILD_BASENAME="cipher"' '-DKBUILD_MODNAME="crypto"' -D__KBUILD_MODNAME=kmod_crypto
LINUX_SOURCES += crypto/cmac
crypto/cmac_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/cmac"' '-DKBUILD_BASENAME="cmac"' '-DKBUILD_MODNAME="cmac"' -D__KBUILD_MODNAME=kmod_cmac
LINUX_SOURCES += crypto/ctr
crypto/ctr_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/ctr"' '-DKBUILD_BASENAME="ctr"' '-DKBUILD_MODNAME="ctr"' -D__KBUILD_MODNAME=kmod_ctr
LINUX_SOURCES += crypto/ecb
crypto/ecb_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/ecb"' '-DKBUILD_BASENAME="ecb"' '-DKBUILD_MODNAME="ecb"' -D__KBUILD_MODNAME=kmod_ecb
LINUX_SOURCES += crypto/gcm
crypto/gcm_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/gcm"' '-DKBUILD_BASENAME="gcm"' '-DKBUILD_MODNAME="gcm"' -D__KBUILD_MODNAME=kmod_gcm
LINUX_SOURCES += crypto/ghash-generic
crypto/ghash-generic_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/ghash-generic"' '-DKBUILD_BASENAME="ghash_generic"' '-DKBUILD_MODNAME="ghash_generic"' -D__KBUILD_MODNAME=kmod_ghash_generic
LINUX_SOURCES += crypto/kpp
crypto/kpp_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/kpp"' '-DKBUILD_BASENAME="kpp"' '-DKBUILD_MODNAME="kpp"' -D__KBUILD_MODNAME=kmod_kpp
LINUX_SOURCES += crypto/lskcipher
crypto/lskcipher_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto_skcipher"' '-DKBUILD_BASENAME="lskcipher"' '-DKBUILD_MODNAME="crypto_skcipher"' -D__KBUILD_MODNAME=kmod_crypto_skcipher
LINUX_SOURCES += crypto/rng
crypto/rng_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/rng"' '-DKBUILD_BASENAME="rng"' '-DKBUILD_MODNAME="rng"' -D__KBUILD_MODNAME=kmod_rng
LINUX_SOURCES += crypto/scatterwalk
crypto/scatterwalk_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto_algapi"' '-DKBUILD_BASENAME="scatterwalk"' '-DKBUILD_MODNAME="crypto_algapi"' -D__KBUILD_MODNAME=kmod_crypto_algapi
LINUX_SOURCES += crypto/scompress
crypto/scompress_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto_acompress"' '-DKBUILD_BASENAME="scompress"' '-DKBUILD_MODNAME="crypto_acompress"' -D__KBUILD_MODNAME=kmod_crypto_acompress
LINUX_SOURCES += crypto/shash
crypto/shash_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto_hash"' '-DKBUILD_BASENAME="shash"' '-DKBUILD_MODNAME="crypto_hash"' -D__KBUILD_MODNAME=kmod_crypto_hash
LINUX_SOURCES += crypto/sig
crypto/sig_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/sig"' '-DKBUILD_BASENAME="sig"' '-DKBUILD_MODNAME="sig"' -D__KBUILD_MODNAME=kmod_sig
LINUX_SOURCES += crypto/skcipher
crypto/skcipher_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/crypto_skcipher"' '-DKBUILD_BASENAME="skcipher"' '-DKBUILD_MODNAME="crypto_skcipher"' -D__KBUILD_MODNAME=kmod_crypto_skcipher
LINUX_SOURCES += crypto/testmgr
crypto/testmgr_FLAGS := -I$(LINUX)/crypto '-DKBUILD_MODFILE="crypto/cryptomgr"' '-DKBUILD_BASENAME="testmgr"' '-DKBUILD_MODNAME="cryptomgr"' -D__KBUILD_MODNAME=kmod_cryptomgr
LINUX_SOURCES += drivers/net/wireless/virtual/mac80211_hwsim
drivers/net/wireless/virtual/mac80211_hwsim_FLAGS := -I$(LINUX)/drivers/net/wireless/virtual '-DKBUILD_MODFILE="drivers/net/wireless/virtual/mac80211_hwsim"' '-DKBUILD_BASENAME="mac80211_hwsim"' '-DKBUILD_MODNAME="mac80211_hwsim"' -D__KBUILD_MODNAME=kmod_mac80211_hwsim
LINUX_SOURCES += drivers/virtio/virtio
drivers/virtio/virtio_FLAGS := -I$(LINUX)/drivers/virtio '-DKBUILD_MODFILE="drivers/virtio/virtio"' '-DKBUILD_BASENAME="virtio"' '-DKBUILD_MODNAME="virtio"' -D__KBUILD_MODNAME=kmod_virtio
LINUX_SOURCES += drivers/virtio/virtio_anchor
drivers/virtio/virtio_anchor_FLAGS := -I$(LINUX)/drivers/virtio '-DKBUILD_MODFILE="drivers/virtio/virtio_anchor"' '-DKBUILD_BASENAME="virtio_anchor"' '-DKBUILD_MODNAME="virtio_anchor"' -D__KBUILD_MODNAME=kmod_virtio_anchor
LINUX_SOURCES += drivers/virtio/virtio_pci_admin_legacy_io
drivers/virtio/virtio_pci_admin_legacy_io_FLAGS := -I$(LINUX)/drivers/virtio '-DKBUILD_MODFILE="drivers/virtio/virtio_pci"' '-DKBUILD_BASENAME="virtio_pci_admin_legacy_io"' '-DKBUILD_MODNAME="virtio_pci"' -D__KBUILD_MODNAME=kmod_virtio_pci
LINUX_SOURCES += drivers/virtio/virtio_pci_common
drivers/virtio/virtio_pci_common_FLAGS := -I$(LINUX)/drivers/virtio '-DKBUILD_MODFILE="drivers/virtio/virtio_pci"' '-DKBUILD_BASENAME="virtio_pci_common"' '-DKBUILD_MODNAME="virtio_pci"' -D__KBUILD_MODNAME=kmod_virtio_pci
LINUX_SOURCES += drivers/virtio/virtio_pci_modern
drivers/virtio/virtio_pci_modern_FLAGS := -I$(LINUX)/drivers/virtio '-DKBUILD_MODFILE="drivers/virtio/virtio_pci"' '-DKBUILD_BASENAME="virtio_pci_modern"' '-DKBUILD_MODNAME="virtio_pci"' -D__KBUILD_MODNAME=kmod_virtio_pci
LINUX_SOURCES += drivers/virtio/virtio_pci_modern_dev
drivers/virtio/virtio_pci_modern_dev_FLAGS := -I$(LINUX)/drivers/virtio '-DKBUILD_MODFILE="drivers/virtio/virtio_pci_modern_dev"' '-DKBUILD_BASENAME="virtio_pci_modern_dev"' '-DKBUILD_MODNAME="virtio_pci_modern_dev"' -D__KBUILD_MODNAME=kmod_virtio_pci_modern_dev
LINUX_SOURCES += drivers/virtio/virtio_ring
drivers/virtio/virtio_ring_FLAGS := -I$(LINUX)/drivers/virtio '-DKBUILD_MODFILE="drivers/virtio/virtio_ring"' '-DKBUILD_BASENAME="virtio_ring"' '-DKBUILD_MODNAME="virtio_ring"' -D__KBUILD_MODNAME=kmod_virtio_ring
LINUX_SOURCES += lib/bitmap
lib/bitmap_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/bitmap"' '-DKBUILD_BASENAME="bitmap"' '-DKBUILD_MODNAME="bitmap"' -D__KBUILD_MODNAME=kmod_bitmap
LINUX_SOURCES += lib/crc/crc32-main
lib/crc/crc32-main_FLAGS := -I$(LINUX)/lib/crc '-DKBUILD_MODFILE="lib/crc/crc32"' '-DKBUILD_BASENAME="crc32_main"' '-DKBUILD_MODNAME="crc32"' -D__KBUILD_MODNAME=kmod_crc32
LINUX_SOURCES += lib/crypto/aes
lib/crypto/aes_FLAGS := -I$(LINUX)/lib/crypto '-DKBUILD_MODFILE="lib/crypto/libaes"' '-DKBUILD_BASENAME="aes"' '-DKBUILD_MODNAME="libaes"' -D__KBUILD_MODNAME=kmod_libaes
LINUX_SOURCES += lib/crypto/arc4
lib/crypto/arc4_FLAGS := -I$(LINUX)/lib/crypto '-DKBUILD_MODFILE="lib/crypto/libarc4"' '-DKBUILD_BASENAME="arc4"' '-DKBUILD_MODNAME="libarc4"' -D__KBUILD_MODNAME=kmod_libarc4
LINUX_SOURCES += lib/crypto/gf128mul
lib/crypto/gf128mul_FLAGS := -I$(LINUX)/lib/crypto '-DKBUILD_MODFILE="lib/crypto/gf128mul"' '-DKBUILD_BASENAME="gf128mul"' '-DKBUILD_MODNAME="gf128mul"' -D__KBUILD_MODNAME=kmod_gf128mul
LINUX_SOURCES += lib/crypto/memneq
lib/crypto/memneq_FLAGS := -I$(LINUX)/lib/crypto '-DKBUILD_MODFILE="lib/crypto/libcryptoutils"' '-DKBUILD_BASENAME="memneq"' '-DKBUILD_MODNAME="libcryptoutils"' -D__KBUILD_MODNAME=kmod_libcryptoutils
LINUX_SOURCES += lib/crypto/utils
lib/crypto/utils_FLAGS := -I$(LINUX)/lib/crypto '-DKBUILD_MODFILE="lib/crypto/libcryptoutils"' '-DKBUILD_BASENAME="utils"' '-DKBUILD_MODNAME="libcryptoutils"' -D__KBUILD_MODNAME=kmod_libcryptoutils
LINUX_SOURCES += lib/ctype
lib/ctype_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/ctype"' '-DKBUILD_BASENAME="ctype"' '-DKBUILD_MODNAME="ctype"' -D__KBUILD_MODNAME=kmod_ctype
LINUX_SOURCES += lib/find_bit
lib/find_bit_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/find_bit"' '-DKBUILD_BASENAME="find_bit"' '-DKBUILD_MODNAME="find_bit"' -D__KBUILD_MODNAME=kmod_find_bit
LINUX_SOURCES += lib/hexdump
lib/hexdump_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/hexdump"' '-DKBUILD_BASENAME="hexdump"' '-DKBUILD_MODNAME="hexdump"' -D__KBUILD_MODNAME=kmod_hexdump
LINUX_SOURCES += lib/hweight
lib/hweight_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/hweight"' '-DKBUILD_BASENAME="hweight"' '-DKBUILD_MODNAME="hweight"' -D__KBUILD_MODNAME=kmod_hweight
LINUX_SOURCES += lib/idr
lib/idr_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/idr"' '-DKBUILD_BASENAME="idr"' '-DKBUILD_MODNAME="idr"' -D__KBUILD_MODNAME=kmod_idr
LINUX_SOURCES += lib/kasprintf
lib/kasprintf_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/kasprintf"' '-DKBUILD_BASENAME="kasprintf"' '-DKBUILD_MODNAME="kasprintf"' -D__KBUILD_MODNAME=kmod_kasprintf
LINUX_SOURCES += lib/kstrtox
lib/kstrtox_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/kstrtox"' '-DKBUILD_BASENAME="kstrtox"' '-DKBUILD_MODNAME="kstrtox"' -D__KBUILD_MODNAME=kmod_kstrtox
LINUX_SOURCES += lib/list_sort
lib/list_sort_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/list_sort"' '-DKBUILD_BASENAME="list_sort"' '-DKBUILD_MODNAME="list_sort"' -D__KBUILD_MODNAME=kmod_list_sort
LINUX_SOURCES += lib/math/gcd
lib/math/gcd_FLAGS := -I$(LINUX)/lib/math '-DKBUILD_MODFILE="lib/math/gcd"' '-DKBUILD_BASENAME="gcd"' '-DKBUILD_MODNAME="gcd"' -D__KBUILD_MODNAME=kmod_gcd
LINUX_SOURCES += lib/nlattr
lib/nlattr_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/nlattr"' '-DKBUILD_BASENAME="nlattr"' '-DKBUILD_MODNAME="nlattr"' -D__KBUILD_MODNAME=kmod_nlattr
LINUX_SOURCES += lib/radix-tree
lib/radix-tree_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/radix-tree"' '-DKBUILD_BASENAME="radix_tree"' '-DKBUILD_MODNAME="radix_tree"' -D__KBUILD_MODNAME=kmod_radix_tree
LINUX_SOURCES += lib/rbtree
lib/rbtree_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/rbtree"' '-DKBUILD_BASENAME="rbtree"' '-DKBUILD_MODNAME="rbtree"' -D__KBUILD_MODNAME=kmod_rbtree
LINUX_SOURCES += lib/refcount
lib/refcount_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/refcount"' '-DKBUILD_BASENAME="refcount"' '-DKBUILD_MODNAME="refcount"' -D__KBUILD_MODNAME=kmod_refcount
LINUX_SOURCES += lib/rhashtable
lib/rhashtable_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/rhashtable"' '-DKBUILD_BASENAME="rhashtable"' '-DKBUILD_MODNAME="rhashtable"' -D__KBUILD_MODNAME=kmod_rhashtable
LINUX_SOURCES += lib/scatterlist
lib/scatterlist_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/scatterlist"' '-DKBUILD_BASENAME="scatterlist"' '-DKBUILD_MODNAME="scatterlist"' -D__KBUILD_MODNAME=kmod_scatterlist
LINUX_SOURCES += lib/seq_buf
lib/seq_buf_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/seq_buf"' '-DKBUILD_BASENAME="seq_buf"' '-DKBUILD_MODNAME="seq_buf"' -D__KBUILD_MODNAME=kmod_seq_buf
LINUX_SOURCES += lib/sort
lib/sort_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/sort"' '-DKBUILD_BASENAME="sort"' '-DKBUILD_MODNAME="sort"' -D__KBUILD_MODNAME=kmod_sort
LINUX_SOURCES += lib/xarray
lib/xarray_FLAGS := -I$(LINUX)/lib '-DKBUILD_MODFILE="lib/xarray"' '-DKBUILD_BASENAME="xarray"' '-DKBUILD_MODNAME="xarray"' -D__KBUILD_MODNAME=kmod_xarray
LINUX_SOURCES += net/mac80211/aead_api
net/mac80211/aead_api_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="aead_api"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/aes_cmac
net/mac80211/aes_cmac_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="aes_cmac"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/aes_gmac
net/mac80211/aes_gmac_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="aes_gmac"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/agg-rx
net/mac80211/agg-rx_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="agg_rx"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/agg-tx
net/mac80211/agg-tx_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="agg_tx"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/airtime
net/mac80211/airtime_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="airtime"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/cfg
net/mac80211/cfg_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="cfg"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/chan
net/mac80211/chan_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="chan"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/driver-ops
net/mac80211/driver-ops_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="driver_ops"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/eht
net/mac80211/eht_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="eht"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/ethtool
net/mac80211/ethtool_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="ethtool"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/fils_aead
net/mac80211/fils_aead_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="fils_aead"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/he
net/mac80211/he_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="he"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/ht
net/mac80211/ht_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="ht"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/ibss
net/mac80211/ibss_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="ibss"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/iface
net/mac80211/iface_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="iface"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/key
net/mac80211/key_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="key"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/link
net/mac80211/link_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="link"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/main
net/mac80211/main_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="main"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/mlme
net/mac80211/mlme_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="mlme"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/ocb
net/mac80211/ocb_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="ocb"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/offchannel
net/mac80211/offchannel_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="offchannel"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/parse
net/mac80211/parse_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="parse"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/rate
net/mac80211/rate_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="rate"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/rc80211_minstrel_ht
net/mac80211/rc80211_minstrel_ht_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="rc80211_minstrel_ht"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/rx
net/mac80211/rx_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="rx"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/s1g
net/mac80211/s1g_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="s1g"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/scan
net/mac80211/scan_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="scan"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/spectmgmt
net/mac80211/spectmgmt_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="spectmgmt"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/sta_info
net/mac80211/sta_info_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="sta_info"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/status
net/mac80211/status_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="status"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/tdls
net/mac80211/tdls_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="tdls"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/tkip
net/mac80211/tkip_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="tkip"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/trace
net/mac80211/trace_FLAGS := -I$(LINUX)/net/mac80211 -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="trace"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/tx
net/mac80211/tx_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="tx"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/util
net/mac80211/util_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="util"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/vht
net/mac80211/vht_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="vht"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/wbrf
net/mac80211/wbrf_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="wbrf"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/wep
net/mac80211/wep_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="wep"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/wme
net/mac80211/wme_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="wme"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/mac80211/wpa
net/mac80211/wpa_FLAGS := -I$(LINUX)/net/mac80211 '-DKBUILD_MODFILE="net/mac80211/mac80211"' '-DKBUILD_BASENAME="wpa"' '-DKBUILD_MODNAME="mac80211"' -D__KBUILD_MODNAME=kmod_mac80211 -DDEBUG
LINUX_SOURCES += net/wireless/ap
net/wireless/ap_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="ap"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/chan
net/wireless/chan_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="chan"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/core
net/wireless/core_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="core"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/ethtool
net/wireless/ethtool_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="ethtool"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/ibss
net/wireless/ibss_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="ibss"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/mesh
net/wireless/mesh_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="mesh"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/michael-mic
net/wireless/michael-mic_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="michael_mic"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/mlme
net/wireless/mlme_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="mlme"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/nl80211
net/wireless/nl80211_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="nl80211"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/ocb
net/wireless/ocb_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="ocb"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/pmsr
net/wireless/pmsr_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="pmsr"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/radiotap
net/wireless/radiotap_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="radiotap"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/reg
net/wireless/reg_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="reg"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/scan
net/wireless/scan_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="scan"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/sme
net/wireless/sme_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="sme"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/sysfs
net/wireless/sysfs_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="sysfs"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/trace
net/wireless/trace_FLAGS := -I$(LINUX)/net/wireless -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="trace"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
LINUX_SOURCES += net/wireless/util
net/wireless/util_FLAGS := -I$(LINUX)/net/wireless '-DKBUILD_MODFILE="net/wireless/cfg80211"' '-DKBUILD_BASENAME="util"' '-DKBUILD_MODNAME="cfg80211"' -D__KBUILD_MODNAME=kmod_cfg80211
