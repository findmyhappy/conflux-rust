#!/usr/bin/env python3
import datetime
import time
import os
import types
import shutil
from eth_utils import decode_hex

from conflux.messages import GetBlockHeaders, GET_BLOCK_HEADERS_RESPONSE
from test_framework.mininode import start_p2p_connection
from test_framework.test_framework import ConfluxTestFramework
from test_framework.util import assert_equal, connect_nodes, get_peer_addr, wait_until, WaitHandler, \
    initialize_datadir, PortMin, get_datadir_path


class RpcTest(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 2
        self.conf_parameters = {
            "executive_trace": "true",
            "public_rpc_apis": "\"cfx,debug,test,pubsub,trace\"",
            # Disable 1559 for RPC tests temporarily
            "cip1559_transition_height": str(99999999),
            "cip151_transition_height": str(99999999),
            "cip645_transition_height": str(99999999),
        }

    def setup_network(self):
        self.setup_nodes()

    def run_test(self):
        time.sleep(7)
        self._test_sayhello()

        blocks = self.nodes[0].test_generateEmptyBlocks(1)
        self.best_block_hash = blocks[-1] #make_genesis().block_header.hash

        self._test_getblockcount()
        self._test_best_block_hash()
        self._test_getpeerinfo()
        self._test_addlatency()
        self._test_getstatus()
        # self._test_gettransactionreceipt()

        # Test all cases under subfolder
        self._test_subfolder("rpc")

        # Test stop at last
        self._test_stop()

    def _test_subfolder(self, name):
        cur_dir = os.path.dirname(os.path.abspath(__file__))
        sub_dir = os.path.join(cur_dir, name)
        for file in os.listdir(sub_dir):
            if file.startswith("test_") and file.endswith(".py"):
                module_name = file[0:-3]
                module = __import__(name + "." + module_name, fromlist=True)
                self._test_module(module)

    def _test_module(self, module):
        for name in dir(module):
            if name.startswith("Test"):
                obj = getattr(module, name)
                if isinstance(obj, type):
                    self._test_class(name, obj)

    def _test_class(self, class_name, class_type):
        # TODO Clean old nodes
        # Setup a clean node to run each test
        self.stop_nodes()
        for i in range(len(self.nodes)):
            datadir = get_datadir_path(self.options.tmpdir, i)
            shutil.rmtree(datadir)
        old_pos_files = ["initial_nodes.json", "genesis_file", "waypoint_config", "public_key"]
        for f in old_pos_files:
            os.remove(os.path.join(self.options.tmpdir, f))
        shutil.rmtree(os.path.join(self.options.tmpdir, "private_keys"))
        self.nodes = []
        self.add_nodes(1)
        node_index = len(self.nodes) - 1
        initialize_datadir(self.options.tmpdir, node_index, PortMin.n, self.conf_parameters)
        self.start_node(node_index)
        obj = class_type(self.nodes[node_index])

        for name in dir(obj):
            m = getattr(obj, name)
            if type(m) is types.MethodType and name.startswith("test_"):
                self.log.info("Test " + class_name + "." + name)
                m()

    def _test_sayhello(self):
        self.log.info("Test sayhello")
        hello_string = "Hello, world"
        res = self.nodes[0].test_sayHello()
        assert_equal(hello_string, res)

    def _test_getblockcount(self):
        self.log.info("Test getblockcount")
        # TODO test in the future

    def _test_getgoodput(self):
        self.log.info("Test getgoodput")
        # TODO test in the future

    def _test_best_block_hash(self):
        self.log.info("Test best_block_hash")
        res = self.nodes[0].best_block_hash()
        assert_equal(self.best_block_hash, res)

    def _test_getpeerinfo(self):
        self.log.info("Test getpeerinfo")
        connect_nodes(self.nodes, 0, 1)
        res = self.nodes[0].test_getPeerInfo()
        assert_equal(len(res), 1)
        assert_equal(len(self.nodes[1].test_getPeerInfo()), 1)
        assert_equal(res[0]['addr'], get_peer_addr(self.nodes[1]))
        self.nodes[0].test_removeNode(self.nodes[1].key, get_peer_addr(self.nodes[1]))
        try:
            wait_until(lambda: len(self.nodes[0].test_getPeerInfo()) == 0, timeout=10)
        except Exception:
            assert False

    def _test_addlatency(self):
        def on_block_headers(node, _):
            msec = (datetime.datetime.now() - node.start_time).total_seconds() * 1000
            self.log.info("Message arrived after " + str(msec) + "ms")
            # The EventLoop in rust may have a deviation of a maximum of
            # 100ms. This is because the ticker is 100ms by default.
            assert msec >= node.latency_ms - 100

        self.log.info("Test addlatency")
        block_hash = decode_hex(self.nodes[0].test_generateEmptyBlocks(1)[0])
        default_node = start_p2p_connection([self.nodes[0]])[0]
        latency_ms = 1000
        self.nodes[0].test_addLatency(default_node.key, latency_ms)
        default_node.start_time = datetime.datetime.now()
        default_node.latency_ms = latency_ms
        handler = WaitHandler(default_node, GET_BLOCK_HEADERS_RESPONSE, on_block_headers)
        self.nodes[0].p2p.send_protocol_msg(GetBlockHeaders(hashes=[block_hash]))
        handler.wait()

    def _test_getstatus(self):
        self.log.info("Test cfx_getStatus")
        res = self.nodes[0].cfx_getStatus()
        block_count = self.nodes[0].test_getBlockCount()
        assert_equal(hex(block_count), res['blockNumber'])

    def _test_stop(self):
        self.log.info("Test stop")
        try:
            self.nodes[0].test_stop()
            self.nodes[0].test_getPeerInfo()
            assert False
        except Exception:
            pass

    # def _test_gettransactionreceipt(self):
    #     self.log.info("Test checktx")
    #     sk = default_config["GENESIS_PRI_KEY"]
    #     tx = create_transaction(pri_key=sk, value=1000, nonce=1)
    #     assert_equal(checktx(self.nodes[0], tx.hash_hex()), False)
    #     self.nodes[0].p2p.send_protocol_msg(Transactions(transactions=[tx]))

    #     def check_tx():
    #         self.nodes[0].test_generateOneBlock(1)
    #         return checktx(self.nodes[0], tx.hash_hex())
    #     wait_until(check_tx)


if __name__ == "__main__":
    RpcTest().main()
